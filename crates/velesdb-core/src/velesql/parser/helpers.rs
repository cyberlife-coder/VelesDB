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
        return Ok(Value::String(unescape_string_literal(input)));
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

/// Tries to parse a string as `i64` then `u64` (issue #486).
fn try_parse_integer(s: &str) -> Option<Value> {
    s.parse::<i64>()
        .map(Value::Integer)
        .ok()
        .or_else(|| s.parse::<u64>().map(Value::UnsignedInteger).ok())
}

/// Attempts to parse a string as an integer (i64 or u64) or float value.
fn parse_numeric_value(input: &str) -> Result<Value, ParseError> {
    if let Some(int_val) = try_parse_integer(input) {
        return Ok(int_val);
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
        Rule::integer => parse_integer_literal(pair.as_str()),
        Rule::float => parse_float_literal(pair.as_str()),
        Rule::string => Ok(Value::String(unescape_string_literal(pair.as_str()))),
        Rule::boolean => Ok(Value::Boolean(pair.as_str().eq_ignore_ascii_case("true"))),
        Rule::null_value => Ok(Value::Null),
        Rule::parameter => Ok(parse_parameter_value(pair.as_str())),
        _ => Err(ParseError::syntax(0, pair.as_str(), "Unknown value type")),
    }
}

/// Parses a `$name` parameter token into [`Value::Parameter`].
fn parse_parameter_value(raw: &str) -> Value {
    Value::Parameter(raw.trim_start_matches('$').to_string())
}

/// Parses an integer literal string into [`Value::Integer`] or [`Value::UnsignedInteger`].
///
/// Tries `i64` first (covers most integers). Falls back to `u64` for values
/// in the range `(i64::MAX, u64::MAX]` (issue #486).
fn parse_integer_literal(s: &str) -> Result<Value, ParseError> {
    try_parse_integer(s).ok_or_else(|| ParseError::syntax(0, s, "Invalid integer"))
}

/// Parses a float literal string into a [`Value::Float`].
fn parse_float_literal(s: &str) -> Result<Value, ParseError> {
    s.parse::<f64>()
        .map(Value::Float)
        .map_err(|_| ParseError::syntax(0, s, "Invalid float"))
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

/// Strips surrounding single quotes and unescapes SQL-style doubled quotes.
///
/// `'O''Brien'` becomes `O'Brien`. The grammar guarantees the string starts
/// and ends with `'` and is at least 2 chars long (atomic rule).
pub(crate) fn unescape_string_literal(raw: &str) -> String {
    raw[1..raw.len() - 1].replace("''", "'")
}

/// Extracts key-value pairs from a list of pest pairs.
///
/// Iterates `list_pair.into_inner()`, filters by `item_rule`, and applies
/// `extractor` to each matching pair. Used by both DDL and DML parsers
/// to avoid structural duplication in option/field list parsing.
///
/// # Errors
///
/// Returns [`ParseError`] if any individual `extractor` call fails.
pub(crate) fn extract_key_value_list<T>(
    list_pair: pest::iterators::Pair<'_, super::Rule>,
    item_rule: super::Rule,
    extractor: impl Fn(pest::iterators::Pair<'_, super::Rule>) -> Result<T, ParseError>,
) -> Result<Vec<T>, ParseError> {
    list_pair
        .into_inner()
        .filter(|p| p.as_rule() == item_rule)
        .map(extractor)
        .collect()
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
#[path = "helpers_tests.rs"]
mod tests;
