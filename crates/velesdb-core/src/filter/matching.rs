//! Condition matching logic and helper functions.

use super::Condition;
use serde_json::Value;

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
    let (text, pattern) = if case_insensitive {
        (text.to_lowercase(), pattern.to_lowercase())
    } else {
        (text.to_string(), pattern.to_string())
    };

    like_match_impl(text.as_bytes(), pattern.as_bytes())
}

/// Recursive implementation of LIKE matching using dynamic programming approach.
fn like_match_impl(text: &[u8], pattern: &[u8]) -> bool {
    let m = text.len();
    let n = pattern.len();

    // dp[i][j] = true if text[0..i] matches pattern[0..j]
    let mut dp = vec![vec![false; n + 1]; m + 1];

    // Empty pattern matches empty text
    dp[0][0] = true;

    // Handle patterns starting with % (can match empty string)
    let mut j = 0;
    while j < n {
        if pattern[j] == b'%' {
            dp[0][j + 1] = dp[0][j];
            j += 1;
        } else if pattern[j] == b'\\' && j + 1 < n {
            // Escaped character - skip both
            break;
        } else {
            break;
        }
    }

    let mut pi = 0;
    while pi < n {
        let (pat_char, pat_len) = if pattern[pi] == b'\\' && pi + 1 < n {
            // Escaped character: \% or \_
            (Some(pattern[pi + 1]), 2)
        } else if pattern[pi] == b'%' {
            (None, 1) // % wildcard
        } else if pattern[pi] == b'_' {
            (Some(0), 1) // _ wildcard (0 means "match any single char")
        } else {
            (Some(pattern[pi]), 1)
        };

        for ti in 0..=m {
            match pat_char {
                None => {
                    // % matches zero or more characters
                    // dp[ti][pi+1] is true if any dp[k][pi] is true for k <= ti
                    if ti == 0 {
                        dp[ti][pi + pat_len] = dp[ti][pi];
                    } else {
                        dp[ti][pi + pat_len] = dp[ti][pi] || dp[ti - 1][pi + pat_len];
                    }
                }
                Some(0) => {
                    // _ matches exactly one character
                    if ti > 0 {
                        dp[ti][pi + pat_len] = dp[ti - 1][pi];
                    }
                }
                Some(c) => {
                    // Literal character match
                    if ti > 0 && text[ti - 1] == c {
                        dp[ti][pi + pat_len] = dp[ti - 1][pi];
                    }
                }
            }
        }

        pi += pat_len;
    }

    dp[m][n]
}
