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
