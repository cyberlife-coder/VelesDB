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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // TDD Tests for Condition - Equality
    // =========================================================================

    #[test]
    fn test_filter_equality_string() {
        // Arrange
        let filter = Filter::new(Condition::eq("category", "tech"));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_string_no_match() {
        // Arrange
        let filter = Filter::new(Condition::eq("category", "tech"));
        let payload = json!({"category": "science", "price": 100});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_number() {
        // Arrange
        let filter = Filter::new(Condition::eq("price", 100));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_boolean() {
        // Arrange
        let filter = Filter::new(Condition::eq("active", true));
        let payload = json!({"active": true});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_missing_field() {
        // Arrange
        let filter = Filter::new(Condition::eq("missing", "value"));
        let payload = json!({"category": "tech"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Not Equal
    // =========================================================================

    #[test]
    fn test_filter_not_equal() {
        // Arrange
        let filter = Filter::new(Condition::neq("status", "deleted"));
        let payload = json!({"status": "active"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_not_equal_same_value() {
        // Arrange
        let filter = Filter::new(Condition::neq("status", "deleted"));
        let payload = json!({"status": "deleted"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Range (gt, gte, lt, lte)
    // =========================================================================

    #[test]
    fn test_filter_greater_than() {
        // Arrange
        let filter = Filter::new(Condition::gt("price", 50));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_greater_than_equal_boundary() {
        // Arrange
        let filter = Filter::new(Condition::gt("price", 100));
        let payload = json!({"price": 100});

        // Act & Assert - should NOT match (gt, not gte)
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_greater_than_or_equal() {
        // Arrange
        let filter = Filter::new(Condition::gte("price", 100));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_less_than() {
        // Arrange
        let filter = Filter::new(Condition::lt("price", 200));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_less_than_or_equal() {
        // Arrange
        let filter = Filter::new(Condition::lte("price", 100));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - IN
    // =========================================================================

    #[test]
    fn test_filter_in_array() {
        // Arrange
        let filter = Filter::new(Condition::is_in(
            "category",
            vec![json!("tech"), json!("science"), json!("art")],
        ));
        let payload = json!({"category": "tech"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_in_array_no_match() {
        // Arrange
        let filter = Filter::new(Condition::is_in(
            "category",
            vec![json!("tech"), json!("science")],
        ));
        let payload = json!({"category": "sports"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Contains
    // =========================================================================

    #[test]
    fn test_filter_contains() {
        // Arrange
        let filter = Filter::new(Condition::contains("title", "rust"));
        let payload = json!({"title": "Learning rust programming"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_contains_no_match() {
        // Arrange
        let filter = Filter::new(Condition::contains("title", "python"));
        let payload = json!({"title": "Learning rust programming"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Null checks
    // =========================================================================

    #[test]
    fn test_filter_is_null() {
        // Arrange
        let filter = Filter::new(Condition::is_null("deleted_at"));
        let payload = json!({"name": "test", "deleted_at": null});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_is_null_missing_field() {
        // Arrange
        let filter = Filter::new(Condition::is_null("deleted_at"));
        let payload = json!({"name": "test"});

        // Act & Assert - missing field is considered null
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_is_not_null() {
        // Arrange
        let filter = Filter::new(Condition::is_not_null("name"));
        let payload = json!({"name": "test"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Logical operators
    // =========================================================================

    #[test]
    fn test_filter_and() {
        // Arrange
        let filter = Filter::new(Condition::and(vec![
            Condition::eq("category", "tech"),
            Condition::gt("price", 50),
        ]));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_and_partial_match() {
        // Arrange
        let filter = Filter::new(Condition::and(vec![
            Condition::eq("category", "tech"),
            Condition::gt("price", 200), // won't match
        ]));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert - AND requires all conditions to match
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_or() {
        // Arrange
        let filter = Filter::new(Condition::or(vec![
            Condition::eq("category", "tech"),
            Condition::eq("category", "science"),
        ]));
        let payload = json!({"category": "science"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_or_no_match() {
        // Arrange
        let filter = Filter::new(Condition::or(vec![
            Condition::eq("category", "tech"),
            Condition::eq("category", "science"),
        ]));
        let payload = json!({"category": "sports"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_not() {
        // Arrange
        let filter = Filter::new(Condition::not(Condition::eq("status", "deleted")));
        let payload = json!({"status": "active"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_not_negates() {
        // Arrange
        let filter = Filter::new(Condition::not(Condition::eq("status", "active")));
        let payload = json!({"status": "active"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Nested Fields
    // =========================================================================

    #[test]
    fn test_filter_nested_field() {
        // Arrange
        let filter = Filter::new(Condition::eq("metadata.author", "John"));
        let payload = json!({
            "title": "My Document",
            "metadata": {
                "author": "John",
                "year": 2024
            }
        });

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_deeply_nested_field() {
        // Arrange
        let filter = Filter::new(Condition::eq("a.b.c.d", "value"));
        let payload = json!({
            "a": {
                "b": {
                    "c": {
                        "d": "value"
                    }
                }
            }
        });

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Complex Combined Filters
    // =========================================================================

    #[test]
    fn test_filter_complex_combined() {
        // Arrange - (category = "tech" AND price > 50) OR featured = true
        let filter = Filter::new(Condition::or(vec![
            Condition::and(vec![
                Condition::eq("category", "tech"),
                Condition::gt("price", 50),
            ]),
            Condition::eq("featured", true),
        ]));

        // Act & Assert
        let tech_expensive = json!({"category": "tech", "price": 100, "featured": false});
        assert!(filter.matches(&tech_expensive));

        let featured = json!({"category": "art", "price": 10, "featured": true});
        assert!(filter.matches(&featured));

        let no_match = json!({"category": "art", "price": 10, "featured": false});
        assert!(!filter.matches(&no_match));
    }

    // =========================================================================
    // TDD Tests for Serialization
    // =========================================================================

    #[test]
    fn test_filter_serialization() {
        // Arrange
        let filter = Filter::new(Condition::eq("category", "tech"));

        // Act
        let json = serde_json::to_string(&filter).unwrap();
        let deserialized: Filter = serde_json::from_str(&json).unwrap();

        // Assert
        let payload = json!({"category": "tech"});
        assert!(deserialized.matches(&payload));
    }

    #[test]
    fn test_filter_complex_serialization() {
        // Arrange
        let filter = Filter::new(Condition::and(vec![
            Condition::eq("category", "tech"),
            Condition::gt("price", 100),
        ]));

        // Act
        let json = serde_json::to_string(&filter).unwrap();
        let deserialized: Filter = serde_json::from_str(&json).unwrap();

        // Assert
        let payload = json!({"category": "tech", "price": 150});
        assert!(deserialized.matches(&payload));
    }
}
