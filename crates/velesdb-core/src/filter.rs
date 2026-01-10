//! Metadata filtering for vector search.
//!
//! This module provides a flexible filtering system for narrowing down
//! vector search results based on metadata conditions.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use velesdb_core::filter::{Filter, Condition};
//!
//! // Simple equality filter
//! let filter = Filter::new(Condition::eq("category", "tech"));
//!
//! // Combined filters
//! let filter = Filter::new(Condition::and(vec![
//!     Condition::eq("category", "tech"),
//!     Condition::gt("price", 100),
//! ]));
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A filter for metadata-based search refinement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    /// The root condition of the filter.
    pub condition: Condition,
}

impl Filter {
    /// Creates a new filter with the given condition.
    #[must_use]
    pub fn new(condition: Condition) -> Self {
        Self { condition }
    }

    /// Evaluates the filter against a payload.
    ///
    /// Returns `true` if the payload matches the filter conditions.
    #[must_use]
    pub fn matches(&self, payload: &Value) -> bool {
        self.condition.matches(payload)
    }
}

/// A condition for filtering metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    /// Equality comparison: field == value
    Eq {
        /// Field name (supports dot notation for nested fields)
        field: String,
        /// Value to compare against
        value: Value,
    },
    /// Not equal comparison: field != value
    Neq {
        /// Field name
        field: String,
        /// Value to compare against
        value: Value,
    },
    /// Greater than comparison: field > value
    Gt {
        /// Field name
        field: String,
        /// Value to compare against
        value: Value,
    },
    /// Greater than or equal comparison: field >= value
    Gte {
        /// Field name
        field: String,
        /// Value to compare against
        value: Value,
    },
    /// Less than comparison: field < value
    Lt {
        /// Field name
        field: String,
        /// Value to compare against
        value: Value,
    },
    /// Less than or equal comparison: field <= value
    Lte {
        /// Field name
        field: String,
        /// Value to compare against
        value: Value,
    },
    /// Check if field value is in a list
    In {
        /// Field name
        field: String,
        /// List of values to check against
        values: Vec<Value>,
    },
    /// Check if field contains a substring (for strings)
    Contains {
        /// Field name
        field: String,
        /// Substring to search for
        value: String,
    },
    /// Check if field is null
    IsNull {
        /// Field name
        field: String,
    },
    /// Check if field is not null
    IsNotNull {
        /// Field name
        field: String,
    },
    /// Logical AND of multiple conditions
    And {
        /// Conditions to AND together
        conditions: Vec<Condition>,
    },
    /// Logical OR of multiple conditions
    Or {
        /// Conditions to OR together
        conditions: Vec<Condition>,
    },
    /// Logical NOT of a condition
    Not {
        /// Condition to negate
        condition: Box<Condition>,
    },
    /// SQL LIKE pattern matching (case-sensitive).
    ///
    /// Supports wildcards:
    /// - `%` matches zero or more characters
    /// - `_` matches exactly one character
    /// - `\%` matches a literal `%`
    /// - `\_` matches a literal `_`
    Like {
        /// Field name
        field: String,
        /// Pattern with SQL wildcards
        pattern: String,
    },
    /// SQL ILIKE pattern matching (case-insensitive).
    ///
    /// Same as LIKE but ignores case.
    #[serde(rename = "ilike")]
    ILike {
        /// Field name
        field: String,
        /// Pattern with SQL wildcards
        pattern: String,
    },
}

impl From<crate::velesql::Condition> for Condition {
    #[allow(clippy::too_many_lines)]
    fn from(cond: crate::velesql::Condition) -> Self {
        match cond {
            crate::velesql::Condition::Comparison(cmp) => {
                let value = match cmp.value {
                    crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                    crate::velesql::Value::Float(f) => Value::from(f),
                    crate::velesql::Value::String(s) => Value::String(s),
                    crate::velesql::Value::Boolean(b) => Value::Bool(b),
                    crate::velesql::Value::Null | crate::velesql::Value::Parameter(_) => {
                        Value::Null
                    }
                };
                match cmp.operator {
                    crate::velesql::CompareOp::Eq => Self::eq(cmp.column, value),
                    crate::velesql::CompareOp::NotEq => Self::neq(cmp.column, value),
                    crate::velesql::CompareOp::Gt => Self::Gt {
                        field: cmp.column,
                        value,
                    },
                    crate::velesql::CompareOp::Gte => Self::Gte {
                        field: cmp.column,
                        value,
                    },
                    crate::velesql::CompareOp::Lt => Self::Lt {
                        field: cmp.column,
                        value,
                    },
                    crate::velesql::CompareOp::Lte => Self::Lte {
                        field: cmp.column,
                        value,
                    },
                }
            }
            crate::velesql::Condition::In(inc) => {
                let values = inc
                    .values
                    .into_iter()
                    .map(|v| match v {
                        crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                        crate::velesql::Value::Float(f) => Value::from(f),
                        crate::velesql::Value::String(s) => Value::String(s),
                        crate::velesql::Value::Boolean(b) => Value::Bool(b),
                        crate::velesql::Value::Null | crate::velesql::Value::Parameter(_) => {
                            Value::Null
                        }
                    })
                    .collect();
                Self::In {
                    field: inc.column,
                    values,
                }
            }
            crate::velesql::Condition::IsNull(isn) => {
                if isn.is_null {
                    Self::IsNull { field: isn.column }
                } else {
                    Self::IsNotNull { field: isn.column }
                }
            }
            crate::velesql::Condition::And(left, right) => Self::And {
                conditions: vec![Self::from(*left), Self::from(*right)],
            },
            crate::velesql::Condition::Or(left, right) => Self::Or {
                conditions: vec![Self::from(*left), Self::from(*right)],
            },
            crate::velesql::Condition::Not(inner) => Self::Not {
                condition: Box::new(Self::from(*inner)),
            },
            crate::velesql::Condition::Group(inner) => Self::from(*inner),
            crate::velesql::Condition::VectorSearch(_) => {
                // Vector search is handled separately by the query engine
                Self::And { conditions: vec![] } // Identity for AND
            }
            crate::velesql::Condition::VectorFusedSearch(_) => {
                // Fused vector search is handled separately by the query engine
                Self::And { conditions: vec![] } // Identity for AND
            }
            crate::velesql::Condition::Match(m) => Self::Contains {
                field: m.column,
                value: m.query,
            },
            crate::velesql::Condition::Between(btw) => {
                let low = match btw.low {
                    crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                    crate::velesql::Value::Float(f) => Value::from(f),
                    _ => Value::Null,
                };
                let high = match btw.high {
                    crate::velesql::Value::Integer(i) => Value::Number(i.into()),
                    crate::velesql::Value::Float(f) => Value::from(f),
                    _ => Value::Null,
                };
                Self::And {
                    conditions: vec![
                        Self::Gte {
                            field: btw.column.clone(),
                            value: low,
                        },
                        Self::Lte {
                            field: btw.column,
                            value: high,
                        },
                    ],
                }
            }
            crate::velesql::Condition::Like(lk) => {
                // VelesQL LIKE is case-sensitive (ILIKE support to be added in parser)
                Self::Like {
                    field: lk.column,
                    pattern: lk.pattern,
                }
            }
        }
    }
}

impl Condition {
    /// Creates an equality condition.
    #[must_use]
    pub fn eq(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Eq {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Creates a not-equal condition.
    #[must_use]
    pub fn neq(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Neq {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Creates a greater-than condition.
    #[must_use]
    pub fn gt(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Gt {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Creates a greater-than-or-equal condition.
    #[must_use]
    pub fn gte(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Gte {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Creates a less-than condition.
    #[must_use]
    pub fn lt(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Lt {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Creates a less-than-or-equal condition.
    #[must_use]
    pub fn lte(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Lte {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Creates an IN condition (field value must be in the list).
    #[must_use]
    pub fn is_in(field: impl Into<String>, values: Vec<Value>) -> Self {
        Self::In {
            field: field.into(),
            values,
        }
    }

    /// Creates a contains condition for string fields.
    #[must_use]
    pub fn contains(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Contains {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Creates an is-null condition.
    #[must_use]
    pub fn is_null(field: impl Into<String>) -> Self {
        Self::IsNull {
            field: field.into(),
        }
    }

    /// Creates an is-not-null condition.
    #[must_use]
    pub fn is_not_null(field: impl Into<String>) -> Self {
        Self::IsNotNull {
            field: field.into(),
        }
    }

    /// Creates an AND condition combining multiple conditions.
    #[must_use]
    pub fn and(conditions: Vec<Condition>) -> Self {
        Self::And { conditions }
    }

    /// Creates an OR condition combining multiple conditions.
    #[must_use]
    pub fn or(conditions: Vec<Condition>) -> Self {
        Self::Or { conditions }
    }

    /// Creates a NOT condition negating another condition.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn not(condition: Condition) -> Self {
        Self::Not {
            condition: Box::new(condition),
        }
    }

    /// Creates a LIKE condition for SQL-style pattern matching (case-sensitive).
    ///
    /// # Wildcards
    ///
    /// - `%` matches zero or more characters
    /// - `_` matches exactly one character
    /// - `\%` matches a literal `%`
    /// - `\_` matches a literal `_`
    #[must_use]
    pub fn like(field: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self::Like {
            field: field.into(),
            pattern: pattern.into(),
        }
    }

    /// Creates an ILIKE condition for SQL-style pattern matching (case-insensitive).
    ///
    /// Same as LIKE but ignores case when matching.
    #[must_use]
    pub fn ilike(field: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self::ILike {
            field: field.into(),
            pattern: pattern.into(),
        }
    }

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
