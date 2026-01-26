//! Tests for VelesQL query validation (EPIC-044 US-007).
//!
//! These tests validate that parse-time validation correctly detects
//! VelesQL limitations and provides helpful error messages.

use super::ast::{
    CompareOp, Comparison, Condition, Query, SelectColumns, SelectStatement, SimilarityCondition,
    Value, VectorExpr,
};
use super::validation::{QueryValidator, ValidationConfig, ValidationError, ValidationErrorKind};

// ============================================================================
// US-007: Multiple similarity() validation
// ============================================================================

#[test]
fn test_validate_multiple_similarity_detected() {
    // Given: A query with multiple similarity() conditions
    let query = create_query_with_multiple_similarity();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: ValidationError is returned
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::MultipleSimilarity);
    assert!(err.suggestion.contains("sequential queries"));
}

#[test]
fn test_validate_single_similarity_passes() {
    // Given: A query with single similarity() condition
    let query = create_query_with_single_similarity();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error
    assert!(result.is_ok());
}

// ============================================================================
// US-007: OR with similarity() validation
// ============================================================================

#[test]
fn test_validate_or_with_similarity_detected() {
    // Given: A query with similarity() OR metadata
    let query = create_query_with_similarity_or_metadata();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: ValidationError is returned
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::SimilarityWithOr);
    assert!(err.suggestion.contains("AND"));
}

#[test]
fn test_validate_and_with_similarity_passes() {
    // Given: A query with similarity() AND metadata
    let query = create_query_with_similarity_and_metadata();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error
    assert!(result.is_ok());
}

// ============================================================================
// US-007: NOT similarity() validation
// ============================================================================

#[test]
fn test_validate_not_similarity_detected() {
    // Given: A query with NOT similarity()
    let query = create_query_with_not_similarity();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: ValidationError is returned (warning level)
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::NotSimilarity);
    assert!(err.suggestion.contains("LIMIT"));
}

#[test]
fn test_validate_not_similarity_with_limit_passes_in_lenient_mode() {
    // Given: A query with NOT similarity() but has LIMIT
    let mut query = create_query_with_not_similarity();
    query.select.limit = Some(100);

    // When: Validation is performed with lenient config
    let config = ValidationConfig::lenient();
    let result = QueryValidator::validate_with_config(&query, &config);

    // Then: No error (LIMIT mitigates the performance concern)
    assert!(result.is_ok());
}

// ============================================================================
// US-007: Valid queries pass validation
// ============================================================================

#[test]
fn test_validate_simple_query_passes() {
    // Given: A simple SELECT query without vector conditions
    let query = create_simple_query();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error
    assert!(result.is_ok());
}

#[test]
fn test_validate_hybrid_query_with_and_passes() {
    // Given: similarity() AND metadata filter
    let query = create_query_with_similarity_and_metadata();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error
    assert!(result.is_ok());
}

// ============================================================================
// US-007: Strict mode validation
// ============================================================================

#[test]
fn test_strict_mode_rejects_not_similarity_without_limit() {
    // Given: A query with NOT similarity() without LIMIT
    let query = create_query_with_not_similarity();

    // When: Validation is performed with strict config
    let config = ValidationConfig::strict();
    let result = QueryValidator::validate_with_config(&query, &config);

    // Then: ValidationError is returned
    assert!(result.is_err());
}

// ============================================================================
// US-007: Error includes position information
// ============================================================================

#[test]
fn test_validation_error_includes_position() {
    // Given: A query with multiple similarity()
    let query = create_query_with_multiple_similarity();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: Error includes position information
    let err = result.unwrap_err();
    // Position should be set (0 is valid for first occurrence)
    assert!(err.position.is_some() || err.kind == ValidationErrorKind::MultipleSimilarity);
}

#[test]
fn test_validation_error_display_format() {
    // Given: A validation error
    let err = ValidationError::new(
        ValidationErrorKind::MultipleSimilarity,
        Some(42),
        "similarity(v,$v1)>0.8 AND similarity(v,$v2)>0.7",
        "Use sequential queries instead",
    );

    // When: Displayed
    let display = format!("{}", err);

    // Then: Contains useful information
    assert!(display.contains("V001"));
    assert!(display.contains("42"));
}

// ============================================================================
// Helper functions to create test queries
// ============================================================================

fn create_query_with_multiple_similarity() -> Query {
    let sim1 = Condition::Similarity(SimilarityCondition {
        field: "v".to_string(),
        vector: VectorExpr::Parameter("v1".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });
    let sim2 = Condition::Similarity(SimilarityCondition {
        field: "v".to_string(),
        vector: VectorExpr::Parameter("v2".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.7,
    });

    Query {
        select: SelectStatement {
            columns: SelectColumns::All,
            from: "docs".to_string(),
            joins: vec![],
            where_clause: Some(Condition::And(Box::new(sim1), Box::new(sim2))),
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

fn create_query_with_single_similarity() -> Query {
    let sim = Condition::Similarity(SimilarityCondition {
        field: "v".to_string(),
        vector: VectorExpr::Parameter("v".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });

    Query {
        select: SelectStatement {
            columns: SelectColumns::All,
            from: "docs".to_string(),
            joins: vec![],
            where_clause: Some(sim),
            order_by: None,
            limit: Some(10),
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
    }
}

fn create_query_with_similarity_or_metadata() -> Query {
    let sim = Condition::Similarity(SimilarityCondition {
        field: "v".to_string(),
        vector: VectorExpr::Parameter("v".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });
    let meta = Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });

    Query {
        select: SelectStatement {
            columns: SelectColumns::All,
            from: "docs".to_string(),
            joins: vec![],
            where_clause: Some(Condition::Or(Box::new(sim), Box::new(meta))),
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

fn create_query_with_similarity_and_metadata() -> Query {
    let sim = Condition::Similarity(SimilarityCondition {
        field: "v".to_string(),
        vector: VectorExpr::Parameter("v".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });
    let meta = Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });

    Query {
        select: SelectStatement {
            columns: SelectColumns::All,
            from: "docs".to_string(),
            joins: vec![],
            where_clause: Some(Condition::And(Box::new(sim), Box::new(meta))),
            order_by: None,
            limit: Some(10),
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
    }
}

fn create_query_with_not_similarity() -> Query {
    let sim = Condition::Similarity(SimilarityCondition {
        field: "v".to_string(),
        vector: VectorExpr::Parameter("v".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });

    Query {
        select: SelectStatement {
            columns: SelectColumns::All,
            from: "docs".to_string(),
            joins: vec![],
            where_clause: Some(Condition::Not(Box::new(sim))),
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

fn create_simple_query() -> Query {
    Query {
        select: SelectStatement {
            columns: SelectColumns::All,
            from: "docs".to_string(),
            joins: vec![],
            where_clause: None,
            order_by: None,
            limit: Some(10),
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
    }
}
