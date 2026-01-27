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
    ///
    /// # EPIC-044 US-001: Multiple similarity() with AND is supported
    ///
    /// Multiple similarity() conditions are allowed when combined with AND
    /// (cascade filtering). Only OR combinations are rejected.
    fn validate_condition(
        condition: &Condition,
        _limit: Option<u64>,
        _config: &ValidationConfig,
    ) -> Result<(), ValidationError> {
        // Count similarity conditions
        let similarity_count = Self::count_similarity_conditions(condition);

        // EPIC-044 US-001: Multiple similarity() in OR is rejected (requires union of vector searches)
        // Multiple similarity() in AND is allowed (cascade filtering)
        if similarity_count > 1 && Self::has_multiple_similarity_in_or(condition) {
            return Err(ValidationError::multiple_similarity(
                "Multiple similarity() in OR are not supported. Use AND instead.",
            ));
        }

        // EPIC-044 US-002: similarity() OR metadata IS now supported (union mode)
        // has_similarity_with_or check removed - union execution handles this

        // EPIC-044 US-003: NOT similarity() IS now supported via full scan
        // Only warn if no LIMIT is present (performance concern)
        // Validation passes - execution handles the scan

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

    // EPIC-044 US-002: has_similarity_with_or removed - no longer blocking similarity() OR metadata

    /// Checks if a condition tree contains any vector search condition.
    /// Includes Similarity, VectorSearch (NEAR), and VectorFusedSearch (NEAR_FUSED).
    #[allow(dead_code)] // Keep for potential future validation rules
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
    #[allow(dead_code)] // Keep for potential future validation rules
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

    /// EPIC-044 US-001: Check if multiple similarity() appear under same OR.
    /// Multiple similarity in AND is allowed (cascade), but OR requires union (unsupported).
    fn has_multiple_similarity_in_or(condition: &Condition) -> bool {
        match condition {
            Condition::Or(left, right) => {
                let left_sim = Self::count_similarity_conditions(left);
                let right_sim = Self::count_similarity_conditions(right);
                // Both sides have similarity = union required (unsupported)
                (left_sim > 0 && right_sim > 0)
                    || Self::has_multiple_similarity_in_or(left)
                    || Self::has_multiple_similarity_in_or(right)
            }
            Condition::And(left, right) => {
                // AND is fine, but check nested ORs
                Self::has_multiple_similarity_in_or(left)
                    || Self::has_multiple_similarity_in_or(right)
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::has_multiple_similarity_in_or(inner)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::ast::{
        CompareOp, Comparison, SelectColumns, SelectStatement, SimilarityCondition, Value,
        VectorExpr, VectorSearch,
    };

    fn make_query(where_clause: Option<Condition>) -> Query {
        Query {
            select: SelectStatement {
                distinct: crate::velesql::DistinctMode::None,
                columns: SelectColumns::All,
                from: "test".to_string(),
                joins: vec![],
                where_clause,
                order_by: None,
                limit: None,
                offset: None,
                with_clause: None,
                group_by: None,
                having: None,
                fusion_clause: None,
            },
            compound: None,
        }
    }

    fn make_comparison(col: &str, val: i64) -> Condition {
        Condition::Comparison(Comparison {
            column: col.to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(val),
        })
    }

    fn make_similarity() -> Condition {
        Condition::Similarity(SimilarityCondition {
            field: "embedding".to_string(),
            vector: VectorExpr::Parameter("v".to_string()),
            operator: CompareOp::Gt,
            threshold: 0.8,
        })
    }

    fn make_vector_search() -> Condition {
        Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Parameter("v".to_string()),
        })
    }

    #[test]
    fn test_validation_error_display() {
        let err = ValidationError::multiple_similarity("test");
        let display = format!("{err}");
        assert!(display.contains("V001"));
        assert!(display.contains("sequential queries"));
    }

    #[test]
    fn test_validation_error_display_with_position() {
        let err = ValidationError::new(
            ValidationErrorKind::MultipleSimilarity,
            Some(42),
            "fragment",
            "suggestion",
        );
        let display = format!("{err}");
        assert!(display.contains("position 42"));
    }

    #[test]
    fn test_validation_error_similarity_with_or() {
        let err = ValidationError::similarity_with_or("test OR");
        assert_eq!(err.kind, ValidationErrorKind::SimilarityWithOr);
        assert!(err.suggestion.contains("AND"));
    }

    #[test]
    fn test_validation_error_not_similarity() {
        let err = ValidationError::not_similarity("NOT sim");
        assert_eq!(err.kind, ValidationErrorKind::NotSimilarity);
        assert!(err.suggestion.contains("LIMIT"));
    }

    #[test]
    fn test_validation_error_kind_codes() {
        assert_eq!(ValidationErrorKind::MultipleSimilarity.code(), "V001");
        assert_eq!(ValidationErrorKind::SimilarityWithOr.code(), "V002");
        assert_eq!(ValidationErrorKind::NotSimilarity.code(), "V003");
        assert_eq!(ValidationErrorKind::ReservedKeyword.code(), "V004");
        assert_eq!(ValidationErrorKind::StringEscaping.code(), "V005");
    }

    #[test]
    fn test_validation_error_kind_messages() {
        assert!(ValidationErrorKind::MultipleSimilarity
            .message()
            .contains("Multiple"));
        assert!(ValidationErrorKind::SimilarityWithOr
            .message()
            .contains("OR"));
        assert!(ValidationErrorKind::NotSimilarity
            .message()
            .contains("full scan"));
        assert!(ValidationErrorKind::ReservedKeyword
            .message()
            .contains("escaping"));
        assert!(ValidationErrorKind::StringEscaping
            .message()
            .contains("string"));
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

    #[test]
    fn test_validate_empty_query() {
        let query = make_query(None);
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_simple_comparison() {
        let query = make_query(Some(make_comparison("age", 25)));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_single_similarity() {
        let query = make_query(Some(make_similarity()));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_single_vector_search() {
        let query = make_query(Some(make_vector_search()));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_similarity_and_comparison() {
        let cond = Condition::And(
            Box::new(make_similarity()),
            Box::new(make_comparison("category", 1)),
        );
        let query = make_query(Some(cond));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_multiple_similarity_in_and() {
        // Multiple similarity in AND is allowed (cascade filtering)
        let cond = Condition::And(Box::new(make_similarity()), Box::new(make_similarity()));
        let query = make_query(Some(cond));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_multiple_similarity_in_or_rejected() {
        // Multiple similarity in OR is rejected (requires union)
        let cond = Condition::Or(Box::new(make_similarity()), Box::new(make_similarity()));
        let query = make_query(Some(cond));
        let result = QueryValidator::validate(&query);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().kind,
            ValidationErrorKind::MultipleSimilarity
        );
    }

    #[test]
    fn test_validate_similarity_or_metadata_allowed() {
        // similarity() OR metadata is allowed (union mode)
        let cond = Condition::Or(
            Box::new(make_similarity()),
            Box::new(make_comparison("status", 1)),
        );
        let query = make_query(Some(cond));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_not_similarity_allowed() {
        // NOT similarity() is allowed (full scan)
        let cond = Condition::Not(Box::new(make_similarity()));
        let query = make_query(Some(cond));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_grouped_condition() {
        let cond = Condition::Group(Box::new(make_similarity()));
        let query = make_query(Some(cond));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_nested_and_or() {
        // (sim AND comp) OR comp - allowed
        let inner = Condition::And(
            Box::new(make_similarity()),
            Box::new(make_comparison("a", 1)),
        );
        let cond = Condition::Or(Box::new(inner), Box::new(make_comparison("b", 2)));
        let query = make_query(Some(cond));
        assert!(QueryValidator::validate(&query).is_ok());
    }

    #[test]
    fn test_validate_deeply_nested_multiple_sim_or() {
        // ((sim) OR (sim)) in nested structure - rejected
        let inner_or = Condition::Or(Box::new(make_similarity()), Box::new(make_similarity()));
        let cond = Condition::Group(Box::new(inner_or));
        let query = make_query(Some(cond));
        assert!(QueryValidator::validate(&query).is_err());
    }

    #[test]
    fn test_validate_with_config_lenient() {
        let query = make_query(Some(Condition::Not(Box::new(make_similarity()))));
        let config = ValidationConfig::lenient();
        assert!(QueryValidator::validate_with_config(&query, &config).is_ok());
    }

    #[test]
    fn test_count_similarity_conditions_none() {
        let cond = make_comparison("x", 1);
        assert_eq!(QueryValidator::count_similarity_conditions(&cond), 0);
    }

    #[test]
    fn test_count_similarity_conditions_one() {
        let cond = make_similarity();
        assert_eq!(QueryValidator::count_similarity_conditions(&cond), 1);
    }

    #[test]
    fn test_count_similarity_conditions_multiple() {
        let cond = Condition::And(
            Box::new(make_similarity()),
            Box::new(Condition::Or(
                Box::new(make_vector_search()),
                Box::new(make_comparison("x", 1)),
            )),
        );
        assert_eq!(QueryValidator::count_similarity_conditions(&cond), 2);
    }

    #[test]
    fn test_contains_similarity_true() {
        let cond = Condition::And(
            Box::new(make_comparison("x", 1)),
            Box::new(make_similarity()),
        );
        assert!(QueryValidator::contains_similarity(&cond));
    }

    #[test]
    fn test_contains_similarity_false() {
        let cond = make_comparison("x", 1);
        assert!(!QueryValidator::contains_similarity(&cond));
    }

    #[test]
    fn test_has_not_similarity_true() {
        let cond = Condition::Not(Box::new(make_similarity()));
        assert!(QueryValidator::has_not_similarity(&cond));
    }

    #[test]
    fn test_has_not_similarity_nested() {
        let cond = Condition::And(
            Box::new(make_comparison("x", 1)),
            Box::new(Condition::Not(Box::new(make_similarity()))),
        );
        assert!(QueryValidator::has_not_similarity(&cond));
    }

    #[test]
    fn test_has_not_similarity_false() {
        let cond = make_similarity();
        assert!(!QueryValidator::has_not_similarity(&cond));
    }

    #[test]
    fn test_validation_error_is_error_trait() {
        let err = ValidationError::multiple_similarity("test");
        let _: &dyn std::error::Error = &err;
    }
}
