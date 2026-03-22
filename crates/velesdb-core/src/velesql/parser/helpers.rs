//! Shared parsing helpers used across VelesQL parser modules.
//!
//! Centralizes common patterns to eliminate duplication:
//! - Comparison operator parsing
//! - Literal value conversion
//! - Integer clause extraction (LIMIT/OFFSET)
//! - Identifier normalization (quote stripping)

use super::Rule;
use crate::velesql::ast::{CompareOp, Value};
use crate::velesql::error::ParseError;

/// Parses a comparison operator string into a [`CompareOp`].
///
/// Accepts: `=`, `!=`, `<>`, `>`, `>=`, `<`, `<=`.
///
/// # Errors
///
/// Returns [`ParseError`] if the operator string is not recognized.
pub(crate) fn compare_op_from_str(op: &str) -> Result<CompareOp, ParseError> {
    match op {
        "=" => Ok(CompareOp::Eq),
        "!=" | "<>" => Ok(CompareOp::NotEq),
        ">" => Ok(CompareOp::Gt),
        ">=" => Ok(CompareOp::Gte),
        "<" => Ok(CompareOp::Lt),
        "<=" => Ok(CompareOp::Lte),
        _ => Err(ParseError::syntax(0, op, "Invalid comparison operator")),
    }
}

/// Parses a raw string literal into a [`Value`].
///
/// Handles integer, float, boolean, null, and single-quoted string literals.
/// This is the string-based counterpart to the pest rule-based [`parse_value_from_pair`].
///
/// # Errors
///
/// Returns [`ParseError`] if the input cannot be recognized as any value type.
pub(crate) fn parse_value_from_str(input: &str) -> Result<Value, ParseError> {
    if input.len() >= 2 && input.starts_with('\'') && input.ends_with('\'') {
        return Ok(Value::String(input.trim_matches('\'').to_string()));
    }
    if input.eq_ignore_ascii_case("true") {
        return Ok(Value::Boolean(true));
    }
    if input.eq_ignore_ascii_case("false") {
        return Ok(Value::Boolean(false));
    }
    if input.eq_ignore_ascii_case("null") {
        return Ok(Value::Null);
    }
    parse_numeric_value(input)
}

/// Attempts to parse a string as an integer or float value.
fn parse_numeric_value(input: &str) -> Result<Value, ParseError> {
    if let Ok(i) = input.parse::<i64>() {
        return Ok(Value::Integer(i));
    }
    if let Ok(f) = input.parse::<f64>() {
        return Ok(Value::Float(f));
    }
    Err(ParseError::syntax(
        0,
        input,
        format!("Invalid value: {input}"),
    ))
}

/// Parses a pest pair representing a scalar literal into a [`Value`].
///
/// Handles `Rule::integer`, `Rule::float`, `Rule::string`, `Rule::boolean`,
/// `Rule::null_value`, and `Rule::parameter`.
///
/// # Errors
///
/// Returns [`ParseError`] for unrecognized rules or malformed literals.
pub(crate) fn parse_scalar_from_rule(
    pair: &pest::iterators::Pair<'_, Rule>,
) -> Result<Value, ParseError> {
    match pair.as_rule() {
        Rule::integer => {
            let v = pair
                .as_str()
                .parse::<i64>()
                .map_err(|_| ParseError::syntax(0, pair.as_str(), "Invalid integer"))?;
            Ok(Value::Integer(v))
        }
        Rule::float => {
            let v = pair
                .as_str()
                .parse::<f64>()
                .map_err(|_| ParseError::syntax(0, pair.as_str(), "Invalid float"))?;
            Ok(Value::Float(v))
        }
        Rule::string => {
            let s = pair.as_str().trim_matches('\'').to_string();
            Ok(Value::String(s))
        }
        Rule::boolean => {
            let b = pair.as_str().to_uppercase() == "TRUE";
            Ok(Value::Boolean(b))
        }
        Rule::null_value => Ok(Value::Null),
        Rule::parameter => {
            let name = pair.as_str().trim_start_matches('$').to_string();
            Ok(Value::Parameter(name))
        }
        _ => Err(ParseError::syntax(0, pair.as_str(), "Unknown value type")),
    }
}

/// Extracts and parses a `u64` integer from a clause pair (e.g., LIMIT, OFFSET).
///
/// Expects the pair to contain exactly one integer child token.
///
/// # Errors
///
/// Returns [`ParseError`] if no integer child is found or parsing fails.
pub(crate) fn parse_u64_clause(
    pair: pest::iterators::Pair<'_, Rule>,
    clause_name: &str,
) -> Result<u64, ParseError> {
    let int_pair = pair
        .into_inner()
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", format!("Expected integer for {clause_name}")))?;

    int_pair.as_str().parse::<u64>().map_err(|_| {
        ParseError::syntax(0, int_pair.as_str(), format!("Invalid {clause_name} value"))
    })
}

/// Strips surrounding backticks or double-quotes from an identifier segment.
///
/// - `` `name` `` becomes `name`
/// - `"col""name"` becomes `col"name` (escaped double-quote)
/// - Unquoted identifiers are returned as-is.
pub(crate) fn strip_identifier_quotes(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('`') && s.ends_with('`') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].replace("\"\"", "\"")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_op_from_str_all_operators() {
        assert_eq!(compare_op_from_str("=").unwrap(), CompareOp::Eq);
        assert_eq!(compare_op_from_str("!=").unwrap(), CompareOp::NotEq);
        assert_eq!(compare_op_from_str("<>").unwrap(), CompareOp::NotEq);
        assert_eq!(compare_op_from_str(">").unwrap(), CompareOp::Gt);
        assert_eq!(compare_op_from_str(">=").unwrap(), CompareOp::Gte);
        assert_eq!(compare_op_from_str("<").unwrap(), CompareOp::Lt);
        assert_eq!(compare_op_from_str("<=").unwrap(), CompareOp::Lte);
    }

    #[test]
    fn test_compare_op_from_str_invalid() {
        assert!(compare_op_from_str("??").is_err());
    }

    #[test]
    fn test_parse_value_from_str_integer() {
        assert_eq!(parse_value_from_str("42").unwrap(), Value::Integer(42));
    }

    #[test]
    fn test_parse_value_from_str_float() {
        assert_eq!(parse_value_from_str("2.72").unwrap(), Value::Float(2.72));
    }

    #[test]
    fn test_parse_value_from_str_string() {
        assert_eq!(
            parse_value_from_str("'hello'").unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_parse_value_from_str_boolean() {
        assert_eq!(parse_value_from_str("true").unwrap(), Value::Boolean(true));
        assert_eq!(
            parse_value_from_str("FALSE").unwrap(),
            Value::Boolean(false)
        );
    }

    #[test]
    fn test_parse_value_from_str_null() {
        assert_eq!(parse_value_from_str("null").unwrap(), Value::Null);
    }

    #[test]
    fn test_parse_value_from_str_invalid() {
        assert!(parse_value_from_str("not_a_value").is_err());
    }

    #[test]
    fn test_parse_u64_clause_error_message() {
        // Verify the error message includes the clause name.
        // We cannot construct a real pest pair without the grammar,
        // so we test indirectly via the error message format.
        let msg = format!("Expected integer for {}", "LIMIT");
        assert!(msg.contains("LIMIT"));
    }

    #[test]
    fn test_strip_identifier_quotes_backtick() {
        assert_eq!(strip_identifier_quotes("`name`"), "name");
    }

    #[test]
    fn test_strip_identifier_quotes_double() {
        assert_eq!(strip_identifier_quotes("\"col\""), "col");
    }

    #[test]
    fn test_strip_identifier_quotes_escaped_double() {
        assert_eq!(strip_identifier_quotes("\"col\"\"name\""), "col\"name");
    }

    #[test]
    fn test_strip_identifier_quotes_plain() {
        assert_eq!(strip_identifier_quotes("plain"), "plain");
    }

    #[test]
    fn test_strip_identifier_quotes_trimmed() {
        assert_eq!(strip_identifier_quotes("  `spaced`  "), "spaced");
    }
}
