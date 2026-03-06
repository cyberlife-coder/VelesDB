//! Condition matching logic and helper functions.

use super::Condition;
use crate::metrics::global_guardrails_metrics;
use serde_json::Value;

const LIKE_MAX_PATTERN_BYTES: usize = 4096;
const LIKE_MAX_DYN_OPS: usize = 2_000_000;

impl Condition {
    /// Evaluates the condition against a payload.
    #[must_use]
    pub fn matches(&self, payload: &Value) -> bool {
        match self {
            Self::Eq { field, value } => {
                get_field(payload, field).is_some_and(|v| values_equal(v, value))
            }
            Self::Neq { field, value } => {
                get_field(payload, field).is_none_or(|v| !values_equal(v, value))
            }
            Self::Gt { field, value } => {
                get_field(payload, field).is_some_and(|v| compare_values(v, value) > 0)
            }
            Self::Gte { field, value } => {
                get_field(payload, field).is_some_and(|v| compare_values(v, value) >= 0)
            }
            Self::Lt { field, value } => {
                get_field(payload, field).is_some_and(|v| compare_values(v, value) < 0)
            }
            Self::Lte { field, value } => {
                get_field(payload, field).is_some_and(|v| compare_values(v, value) <= 0)
            }
            Self::In { field, values } => get_field(payload, field)
                .is_some_and(|v| values.iter().any(|val| values_equal(v, val))),
            Self::Contains { field, value } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| s.contains(value.as_str()))),
            Self::IsNull { field } => get_field(payload, field).is_none_or(Value::is_null),
            Self::IsNotNull { field } => get_field(payload, field).is_some_and(|v| !v.is_null()),
            Self::And { conditions } => conditions.iter().all(|c| c.matches(payload)),
            Self::Or { conditions } => conditions.iter().any(|c| c.matches(payload)),
            Self::Not { condition } => !condition.matches(payload),
            Self::Like { field, pattern } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, false))),
            Self::ILike { field, pattern } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, true))),
        }
    }
}

/// Gets a field from a JSON payload, supporting dot notation for nested fields.
fn get_field<'a>(payload: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = payload;
    for part in field.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

/// Compares two JSON values for equality.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => {
            // Compare as f64 for numeric comparison
            a.as_f64()
                .zip(b.as_f64())
                .is_some_and(|(a, b)| (a - b).abs() < f64::EPSILON)
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => a == b,
        (Value::Object(a), Value::Object(b)) => a == b,
        _ => false,
    }
}

/// Compares two JSON values, returning -1, 0, or 1.
/// Returns 0 if values are not comparable.
fn compare_values(a: &Value, b: &Value) -> i32 {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a.partial_cmp(&b).map_or(0, |ord| ord as i32),
            _ => 0,
        },
        (Value::String(a), Value::String(b)) => a.cmp(b) as i32,
        _ => 0,
    }
}

/// SQL LIKE pattern matching implementation.
///
/// Supports:
/// - `%` matches zero or more characters
/// - `_` matches exactly one character
/// - `\%` matches a literal `%`
/// - `\_` matches a literal `_`
///
/// # Arguments
///
/// * `text` - The string to match against
/// * `pattern` - The SQL LIKE pattern
/// * `case_insensitive` - If true, performs case-insensitive matching (ILIKE)
fn like_match(text: &str, pattern: &str, case_insensitive: bool) -> bool {
    if pattern.len() > LIKE_MAX_PATTERN_BYTES {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return false;
    }

    if text.len().saturating_mul(pattern.len().max(1)) > LIKE_MAX_DYN_OPS {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return false;
    }

    let (text, pattern) = if case_insensitive {
        (text.to_lowercase(), pattern.to_lowercase())
    } else {
        (text.to_string(), pattern.to_string())
    };

    like_match_impl(text.as_bytes(), pattern.as_bytes())
}

#[derive(Clone, Copy)]
enum Token {
    AnySeq,
    AnyOne,
    Literal(u8),
}

fn tokenize_like_pattern(pattern: &[u8]) -> Vec<Token> {
    let mut out = Vec::with_capacity(pattern.len());
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' if i + 1 < pattern.len() => {
                out.push(Token::Literal(pattern[i + 1]));
                i += 2;
            }
            b'%' => {
                if !matches!(out.last(), Some(Token::AnySeq)) {
                    out.push(Token::AnySeq);
                }
                i += 1;
            }
            b'_' => {
                out.push(Token::AnyOne);
                i += 1;
            }
            c => {
                out.push(Token::Literal(c));
                i += 1;
            }
        }
    }
    out
}

/// LIKE matching using rolling DP (`O(text_len * token_len)` time, `O(token_len)` memory).
fn like_match_impl(text: &[u8], pattern: &[u8]) -> bool {
    let tokens = tokenize_like_pattern(pattern);
    let n = tokens.len();

    let mut prev = vec![false; n + 1];
    prev[0] = true;
    for (j, tok) in tokens.iter().enumerate() {
        if matches!(tok, Token::AnySeq) {
            prev[j + 1] = prev[j];
        } else {
            break;
        }
    }

    let mut curr = vec![false; n + 1];
    for &ch in text {
        curr.fill(false);
        for (j, tok) in tokens.iter().enumerate() {
            curr[j + 1] = match tok {
                Token::AnySeq => curr[j] || prev[j + 1],
                Token::AnyOne => prev[j],
                Token::Literal(c) => prev[j] && ch == *c,
            };
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}
