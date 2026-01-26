//! Query validation for VelesQL (EPIC-044 US-007).
//!
//! This module provides parse-time validation to detect VelesQL limitations
//! and provide helpful error messages before query execution.
//!
//! # Limitations Detected
//!
//! - **Multiple `similarity()`**: Only one similarity condition per query is supported
//! - **`similarity()` with OR**: OR operators with similarity conditions are not supported
//! - **NOT `similarity()`**: Negated similarity requires full scan (performance warning)
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::velesql::{Parser, QueryValidator};
//!
//! let query = Parser::parse("SELECT * FROM docs WHERE similarity(v,$v)>0.8")?;
//! QueryValidator::validate(&query)?;
//! ```

use std::fmt;

use super::ast::{Condition, Query};

/// Error that occurred during query validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    /// Kind of validation error.
    pub kind: ValidationErrorKind,
    /// Position in the original query (if available).
    pub position: Option<usize>,
    /// The problematic query fragment.
    pub fragment: String,
    /// Human-readable suggestion for fixing the issue.
    pub suggestion: String,
}

impl ValidationError {
    /// Creates a new validation error.
    #[must_use]
    pub fn new(
        kind: ValidationErrorKind,
        position: Option<usize>,
        fragment: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            position,
            fragment: fragment.into(),
            suggestion: suggestion.into(),
        }
    }

    /// Creates a multiple similarity error.
    #[must_use]
    pub fn multiple_similarity(fragment: impl Into<String>) -> Self {
        Self::new(
            ValidationErrorKind::MultipleSimilarity,
            None,
            fragment,
            "Use sequential queries instead of multiple similarity() conditions in one query",
        )
    }

    /// Creates a similarity with OR error.
    #[must_use]
    pub fn similarity_with_or(fragment: impl Into<String>) -> Self {
        Self::new(
            ValidationErrorKind::SimilarityWithOr,
            None,
            fragment,
            "Use AND instead of OR with similarity(), or split into separate queries",
        )
    }

    /// Creates a NOT similarity error.
    #[must_use]
    pub fn not_similarity(fragment: impl Into<String>) -> Self {
        Self::new(
            ValidationErrorKind::NotSimilarity,
            None,
            fragment,
            "NOT similarity() requires full scan. Add LIMIT clause to bound the scan",
        )
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(pos) = self.position {
            write!(
                f,
                "[{}] {} at position {}: {}",
                self.kind.code(),
                self.kind.message(),
                pos,
                self.suggestion
            )
        } else {
            write!(
                f,
                "[{}] {}: {}",
                self.kind.code(),
                self.kind.message(),
                self.suggestion
            )
        }
    }
}

impl std::error::Error for ValidationError {}

/// Kind of validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationErrorKind {
    /// Multiple similarity() conditions in one query (V001).
    MultipleSimilarity,
    /// similarity() used with OR operator (V002).
    SimilarityWithOr,
    /// NOT similarity() detected - performance warning (V003).
    NotSimilarity,
    /// Reserved keyword used without escaping (V004).
    ReservedKeyword,
    /// String escaping issue (V005).
    StringEscaping,
}

impl ValidationErrorKind {
    /// Returns the error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MultipleSimilarity => "V001",
            Self::SimilarityWithOr => "V002",
            Self::NotSimilarity => "V003",
            Self::ReservedKeyword => "V004",
            Self::StringEscaping => "V005",
        }
    }

    /// Returns a human-readable message for this error kind.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::MultipleSimilarity => "Multiple similarity() conditions not supported",
            Self::SimilarityWithOr => "OR operator not supported with similarity()",
            Self::NotSimilarity => "NOT similarity() requires full scan",
            Self::ReservedKeyword => "Reserved keyword requires escaping",
            Self::StringEscaping => "Invalid string escaping",
        }
    }
}

/// Configuration for query validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationConfig {
    /// If true, NOT similarity() without LIMIT is an error.
    /// If false, NOT similarity() with LIMIT is allowed.
    pub strict_not_similarity: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict_not_similarity: true,
        }
    }
}

impl ValidationConfig {
    /// Creates a strict validation config (NOT similarity always errors).
    #[must_use]
    pub fn strict() -> Self {
        Self {
            strict_not_similarity: true,
        }
    }

    /// Creates a lenient validation config (allow NOT similarity with LIMIT).
    #[must_use]
    pub fn lenient() -> Self {
        Self {
            strict_not_similarity: false,
        }
    }
}

/// Query validator for detecting VelesQL limitations.
pub struct QueryValidator;

impl QueryValidator {
    /// Validates a query using default configuration.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if the query uses unsupported features.
    pub fn validate(query: &Query) -> Result<(), ValidationError> {
        Self::validate_with_config(query, &ValidationConfig::default())
    }

    /// Validates a query using custom configuration.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if the query uses unsupported features.
    pub fn validate_with_config(
        query: &Query,
        config: &ValidationConfig,
    ) -> Result<(), ValidationError> {
        // Validate main SELECT's WHERE clause if present
        if let Some(ref condition) = query.select.where_clause {
            Self::validate_condition(condition, query.select.limit, config)?;
        }

        // Validate compound query's WHERE clause if present (UNION, INTERSECT, EXCEPT)
        if let Some(ref compound) = query.compound {
            if let Some(ref condition) = compound.right.where_clause {
                Self::validate_condition(condition, compound.right.limit, config)?;
            }
        }

        Ok(())
    }

    /// Validates a condition tree.
    fn validate_condition(
        condition: &Condition,
        limit: Option<u64>,
        config: &ValidationConfig,
    ) -> Result<(), ValidationError> {
        // Count similarity conditions
        let similarity_count = Self::count_similarity_conditions(condition);
        if similarity_count > 1 {
            return Err(ValidationError::multiple_similarity(
                "Multiple similarity() conditions detected",
            ));
        }

        // Check for OR with similarity
        if Self::has_similarity_with_or(condition) {
            return Err(ValidationError::similarity_with_or(
                "similarity() used with OR operator",
            ));
        }

        // Check for NOT similarity
        if Self::has_not_similarity(condition) {
            // In lenient mode, allow NOT similarity if LIMIT is present
            if config.strict_not_similarity || limit.is_none() {
                return Err(ValidationError::not_similarity("NOT similarity() detected"));
            }
        }

        Ok(())
    }

    /// Counts the number of vector search conditions in a condition tree.
    /// Includes Similarity, VectorSearch (NEAR), and VectorFusedSearch (NEAR_FUSED).
    fn count_similarity_conditions(condition: &Condition) -> usize {
        match condition {
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_) => 1,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::count_similarity_conditions(left) + Self::count_similarity_conditions(right)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::count_similarity_conditions(inner)
            }
            _ => 0,
        }
    }

    /// Checks if the condition tree has similarity with OR.
    fn has_similarity_with_or(condition: &Condition) -> bool {
        match condition {
            Condition::Or(left, right) => {
                // If either side contains similarity, this is invalid
                Self::contains_similarity(left) || Self::contains_similarity(right)
            }
            Condition::And(left, right) => {
                Self::has_similarity_with_or(left) || Self::has_similarity_with_or(right)
            }
            Condition::Not(inner) | Condition::Group(inner) => Self::has_similarity_with_or(inner),
            _ => false,
        }
    }

    /// Checks if a condition tree contains any vector search condition.
    /// Includes Similarity, VectorSearch (NEAR), and VectorFusedSearch (NEAR_FUSED).
    fn contains_similarity(condition: &Condition) -> bool {
        match condition {
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_) => true,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::contains_similarity(left) || Self::contains_similarity(right)
            }
            Condition::Not(inner) | Condition::Group(inner) => Self::contains_similarity(inner),
            _ => false,
        }
    }

    /// Checks if the condition tree has NOT applied to similarity.
    fn has_not_similarity(condition: &Condition) -> bool {
        match condition {
            Condition::Not(inner) => Self::contains_similarity(inner),
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::has_not_similarity(left) || Self::has_not_similarity(right)
            }
            Condition::Group(inner) => Self::has_not_similarity(inner),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_error_display() {
        let err = ValidationError::multiple_similarity("test");
        let display = format!("{err}");
        assert!(display.contains("V001"));
        assert!(display.contains("sequential queries"));
    }

    #[test]
    fn test_validation_error_kind_codes() {
        assert_eq!(ValidationErrorKind::MultipleSimilarity.code(), "V001");
        assert_eq!(ValidationErrorKind::SimilarityWithOr.code(), "V002");
        assert_eq!(ValidationErrorKind::NotSimilarity.code(), "V003");
    }

    #[test]
    fn test_validation_config_default() {
        let config = ValidationConfig::default();
        assert!(config.strict_not_similarity);
    }

    #[test]
    fn test_validation_config_strict() {
        let config = ValidationConfig::strict();
        assert!(config.strict_not_similarity);
    }

    #[test]
    fn test_validation_config_lenient() {
        let config = ValidationConfig::lenient();
        assert!(!config.strict_not_similarity);
    }
}
