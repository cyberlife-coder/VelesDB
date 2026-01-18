//! Builder methods for creating Condition instances.

use super::Condition;
use serde_json::Value;

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
}
