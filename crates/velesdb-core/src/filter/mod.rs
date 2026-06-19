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

mod builders;
mod conversion;
#[cfg(test)]
mod conversion_tests;
mod matching;

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

    /// Deserializes a `Filter` from a JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error string if the JSON structure does not match
    /// the expected filter format.
    pub fn from_json_value(value: serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value).map_err(|e| format!("Invalid filter: {e}"))
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
#[non_exhaustive]
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
    /// Check if an array field contains a specific value.
    ArrayContains {
        /// Field name
        field: String,
        /// Value to search for in the array
        value: Value,
    },
    /// Check if an array field contains at least one of the values.
    ArrayContainsAny {
        /// Field name
        field: String,
        /// Values to search for (at least one must match)
        values: Vec<Value>,
    },
    /// Check if an array field contains all of the values.
    ArrayContainsAll {
        /// Field name
        field: String,
        /// Values that must all be present in the array
        values: Vec<Value>,
    },
    /// Geospatial distance filter: Haversine distance comparison.
    GeoDistance {
        /// Field name containing `GeoPoint` data
        field: String,
        /// Reference latitude in degrees
        lat: f64,
        /// Reference longitude in degrees
        lng: f64,
        /// Comparison operator
        operator: crate::velesql::CompareOp,
        /// Distance threshold in meters
        threshold: f64,
    },
    /// Geospatial bounding box filter: coordinate containment check.
    GeoBbox {
        /// Field name containing `GeoPoint` data
        field: String,
        /// Minimum latitude
        lat_min: f64,
        /// Minimum longitude
        lng_min: f64,
        /// Maximum latitude
        lat_max: f64,
        /// Maximum longitude
        lng_max: f64,
    },
}

#[cfg(test)]
mod condition_type_names_tests {
    use super::Condition;
    use crate::CONDITION_TYPE_NAMES;

    /// Maps every variant to its serde `type` tag via an exhaustive `match`
    /// (no wildcard arm), so adding a variant fails to compile until it is
    /// listed here, which in turn flags the missing `CONDITION_TYPE_NAMES`
    /// entry asserted below.
    fn expected_tag(condition: &Condition) -> &'static str {
        match condition {
            Condition::Eq { .. } => "eq",
            Condition::Neq { .. } => "neq",
            Condition::Gt { .. } => "gt",
            Condition::Gte { .. } => "gte",
            Condition::Lt { .. } => "lt",
            Condition::Lte { .. } => "lte",
            Condition::In { .. } => "in",
            Condition::Contains { .. } => "contains",
            Condition::IsNull { .. } => "is_null",
            Condition::IsNotNull { .. } => "is_not_null",
            Condition::And { .. } => "and",
            Condition::Or { .. } => "or",
            Condition::Not { .. } => "not",
            Condition::Like { .. } => "like",
            Condition::ILike { .. } => "ilike",
            Condition::ArrayContains { .. } => "array_contains",
            Condition::ArrayContainsAny { .. } => "array_contains_any",
            Condition::ArrayContainsAll { .. } => "array_contains_all",
            Condition::GeoDistance { .. } => "geo_distance",
            Condition::GeoBbox { .. } => "geo_bbox",
        }
    }

    /// Comparison / null / membership variants, in declaration order.
    fn comparison_variants() -> Vec<Condition> {
        use serde_json::Value;
        let f = || "f".to_string();
        let cmp = |c: fn(String, Value) -> Condition| c(f(), Value::Null);
        vec![
            cmp(|field, value| Condition::Eq { field, value }),
            cmp(|field, value| Condition::Neq { field, value }),
            cmp(|field, value| Condition::Gt { field, value }),
            cmp(|field, value| Condition::Gte { field, value }),
            cmp(|field, value| Condition::Lt { field, value }),
            cmp(|field, value| Condition::Lte { field, value }),
            Condition::In {
                field: f(),
                values: vec![],
            },
            Condition::Contains {
                field: f(),
                value: String::new(),
            },
            Condition::IsNull { field: f() },
            Condition::IsNotNull { field: f() },
        ]
    }

    /// Logical, pattern, array and geo variants, in declaration order.
    fn logical_and_geo_variants() -> Vec<Condition> {
        use serde_json::Value;
        let f = || "f".to_string();
        vec![
            Condition::And { conditions: vec![] },
            Condition::Or { conditions: vec![] },
            Condition::Not {
                condition: Box::new(Condition::IsNull { field: f() }),
            },
            Condition::Like {
                field: f(),
                pattern: String::new(),
            },
            Condition::ILike {
                field: f(),
                pattern: String::new(),
            },
            Condition::ArrayContains {
                field: f(),
                value: Value::Null,
            },
            Condition::ArrayContainsAny {
                field: f(),
                values: vec![],
            },
            Condition::ArrayContainsAll {
                field: f(),
                values: vec![],
            },
            Condition::GeoDistance {
                field: f(),
                lat: 0.0,
                lng: 0.0,
                operator: crate::velesql::CompareOp::Lt,
                threshold: 0.0,
            },
            Condition::GeoBbox {
                field: f(),
                lat_min: 0.0,
                lng_min: 0.0,
                lat_max: 0.0,
                lng_max: 0.0,
            },
        ]
    }

    #[test]
    fn condition_type_names_match_serde_tags_in_order() {
        let mut variants = comparison_variants();
        variants.extend(logical_and_geo_variants());
        assert_eq!(variants.len(), CONDITION_TYPE_NAMES.len());
        for (i, variant) in variants.iter().enumerate() {
            let serialized = serde_json::to_value(variant).expect("serialize condition");
            let serde_tag = serialized["type"].as_str().expect("type tag present");
            assert_eq!(serde_tag, expected_tag(variant));
            assert_eq!(CONDITION_TYPE_NAMES[i], serde_tag);
        }
    }
}
