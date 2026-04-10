//! MATCH clause parser for graph pattern matching.
//!
//! Graph pattern parsing (node, relationship, path patterns) lives in the
//! sibling [`super::match_patterns`] module. This module handles the top-level
//! MATCH clause orchestration, WHERE condition parsing, RETURN clause parsing,
//! and shared string-scanning utilities.

use super::helpers::{compare_op_from_str, parse_value_from_str};
use crate::velesql::ast::{Comparison, Condition, Value};
use crate::velesql::error::ParseError;
use crate::velesql::graph_pattern::{MatchClause, ReturnClause, ReturnItem};
use std::collections::HashMap;

// Re-export pattern parsers so existing callers (tests, external modules)
// continue to find them at `match_clause::parse_node_pattern` etc.
pub use super::match_patterns::{parse_node_pattern, parse_relationship_pattern};

use super::match_patterns::parse_pattern_list;

/// Parses a complete MATCH clause.
///
/// # Errors
///
/// Returns [`ParseError`] when the input is not a valid MATCH query
/// (missing required clauses or malformed pattern/WHERE segments).
pub fn parse_match_clause(input: &str) -> Result<MatchClause, ParseError> {
    let input = input.trim();
    if !input.to_uppercase().starts_with("MATCH ") {
        return Err(ParseError::syntax(0, input, "Expected MATCH keyword"));
    }
    let after_match = input[6..].trim_start();
    let return_pos = find_keyword(after_match, "RETURN")
        .ok_or_else(|| ParseError::syntax(input.len(), input, "Expected RETURN clause"))?;
    let where_pos = find_keyword(&after_match[..return_pos], "WHERE");
    let pattern_end = where_pos.unwrap_or(return_pos);
    let pattern_str = after_match[..pattern_end].trim();
    if pattern_str.is_empty() {
        return Err(ParseError::syntax(6, input, "Expected pattern after MATCH"));
    }
    let patterns = parse_pattern_list(pattern_str)?;
    let where_clause = extract_where_clause(after_match, where_pos, return_pos, input)?;
    let return_clause = parse_return_clause(after_match[return_pos + 6..].trim());
    Ok(MatchClause {
        patterns,
        where_clause,
        return_clause,
    })
}

/// Extracts and parses the optional WHERE clause between the pattern and RETURN.
fn extract_where_clause(
    after_match: &str,
    where_pos: Option<usize>,
    return_pos: usize,
    input: &str,
) -> Result<Option<Condition>, ParseError> {
    let Some(wp) = where_pos else {
        return Ok(None);
    };
    // Validate slice bounds: wp + 5 (after "WHERE") must be <= return_pos
    let where_end = wp + 5;
    if where_end > return_pos {
        return Err(ParseError::syntax(wp, input, "Empty WHERE condition"));
    }
    let condition = parse_where_condition(after_match[where_end..return_pos].trim())?;
    Ok(Some(condition))
}

/// Splits `inner` at the first `{...}` block, returning `(text_before_brace, parsed_properties)`.
///
/// If no braces are present, returns `(inner, empty_map)`.
/// `error_context` is used in brace-mismatch error messages (e.g. "node pattern").
pub(super) fn split_with_braces<'a>(
    inner: &'a str,
    error_source: &str,
    error_context: &str,
) -> Result<(&'a str, HashMap<String, Value>), ParseError> {
    let Some(ps) = inner.find('{') else {
        return Ok((inner, HashMap::new()));
    };
    let pe = inner
        .rfind('}')
        .ok_or_else(|| ParseError::syntax(ps, error_source, "Expected '}'"))?;
    if pe <= ps {
        return Err(ParseError::syntax(
            ps,
            error_source,
            format!("Mismatched braces in {error_context}"),
        ));
    }
    Ok((inner[..ps].trim(), parse_properties(&inner[ps + 1..pe])?))
}

/// Splits properties respecting string literals (commas inside quotes are preserved).
fn parse_properties(input: &str) -> Result<HashMap<String, Value>, ParseError> {
    let mut props = HashMap::new();
    let mut in_string = false;
    let mut start = 0;

    for (i, ch) in input.char_indices() {
        if ch == '\'' {
            in_string = !in_string;
        } else if ch == ',' && !in_string {
            insert_property(input[start..i].trim(), &mut props)?;
            start = i + 1;
        }
    }

    insert_property(input[start..].trim(), &mut props)?;
    Ok(props)
}

/// Parses a single `key: value` property and inserts it into the map.
fn insert_property(prop: &str, props: &mut HashMap<String, Value>) -> Result<(), ParseError> {
    if let Some(c) = prop.find(':') {
        props.insert(
            prop[..c].trim().to_string(),
            parse_value(prop[c + 1..].trim())?,
        );
    }
    Ok(())
}

fn parse_value(input: &str) -> Result<Value, ParseError> {
    parse_value_from_str(input)
}

/// Operator tokens to scan for, ordered longest-first to avoid ambiguous matches.
const WHERE_OPERATORS: &[(&str, usize)] = &[
    ("!=", 2),
    ("<>", 2),
    (">=", 2),
    ("<=", 2),
    (">", 1),
    ("<", 1),
    ("=", 1),
];

fn parse_where_condition(input: &str) -> Result<Condition, ParseError> {
    // Order matters: check multi-char operators before single-char ones.
    // Use string-literal-aware search to avoid matching operators inside quotes.
    let (col, op, vs) = find_where_operator(input)?;
    let operator = compare_op_from_str(op)?;
    Ok(Condition::Comparison(Comparison {
        column: col.trim().to_string(),
        operator,
        value: parse_value(vs)?,
    }))
}

/// Finds the first comparison operator outside string literals and splits the
/// input into `(column, operator_str, value_str)`.
fn find_where_operator(input: &str) -> Result<(&str, &str, &str), ParseError> {
    for &(op_str, op_len) in WHERE_OPERATORS {
        if let Some(p) = find_operator(input, op_str) {
            return Ok((&input[..p], op_str, input[p + op_len..].trim()));
        }
    }
    Err(ParseError::syntax(0, input, "Invalid WHERE"))
}

/// Finds an operator in the input string, respecting string literal boundaries.
/// Returns the byte position of the operator, or None if not found outside quotes.
fn find_operator(input: &str, op: &str) -> Option<usize> {
    scan_outside_quotes(input, op, false)
}

fn parse_return_clause(input: &str) -> ReturnClause {
    let (is, limit) = if let Some(lp) = find_keyword(input, "LIMIT") {
        (&input[..lp], input[lp + 5..].trim().parse().ok())
    } else {
        (input, None)
    };
    let items = is
        .split(',')
        .map(|i| {
            let i = i.trim();
            if let Some(ap) = find_keyword(i, "AS") {
                ReturnItem {
                    expression: i[..ap].trim().to_string(),
                    alias: Some(i[ap + 2..].trim().to_string()),
                }
            } else {
                ReturnItem {
                    expression: i.to_string(),
                    alias: None,
                }
            }
        })
        .collect();
    ReturnClause {
        items,
        order_by: None,
        limit,
    }
}

/// Finds a keyword in the input string, respecting string literal boundaries.
/// Uses ASCII-only case-insensitive matching to avoid Unicode index issues.
pub(super) fn find_keyword(input: &str, kw: &str) -> Option<usize> {
    scan_outside_quotes(input, kw, true)
}

/// Scans `input` for `needle`, skipping regions inside single-quoted string literals.
///
/// When `word_boundary` is true, the match must be surrounded by non-word characters
/// (ASCII alphanumeric or `_`), and matching is ASCII case-insensitive (for SQL keywords).
/// When `word_boundary` is false, the match is exact and byte-level (for operators).
fn scan_outside_quotes(input: &str, needle: &str, word_boundary: bool) -> Option<usize> {
    let bytes = input.as_bytes();
    let needle_bytes = needle.as_bytes();
    let needle_len = needle_bytes.len();

    if needle_len == 0 || bytes.len() < needle_len {
        return None;
    }

    let mut in_string = false;
    let mut i = 0;

    while i <= bytes.len() - needle_len {
        if bytes[i] == b'\'' {
            in_string = !in_string;
            i += 1;
            continue;
        }

        if in_string {
            i += 1;
            continue;
        }

        if needle_matches_at(bytes, needle_bytes, i, word_boundary) {
            return Some(i);
        }

        i += 1;
    }

    None
}

/// Checks whether `needle` matches `bytes` at position `pos`.
///
/// When `word_boundary` is true, the match is case-insensitive and must be
/// surrounded by non-word characters. Otherwise, exact byte comparison is used.
fn needle_matches_at(bytes: &[u8], needle: &[u8], pos: usize, word_boundary: bool) -> bool {
    let needle_len = needle.len();
    let content_matches = if word_boundary {
        bytes[pos..pos + needle_len]
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    } else {
        &bytes[pos..pos + needle_len] == needle
    };

    if !content_matches {
        return false;
    }

    if !word_boundary {
        return true;
    }

    let before_ok = pos == 0 || !is_word_byte(bytes[pos - 1]);
    let after_ok = pos + needle_len >= bytes.len() || !is_word_byte(bytes[pos + needle_len]);
    before_ok && after_ok
}

/// Returns true if `b` is an ASCII alphanumeric byte or underscore.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
