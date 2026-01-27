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
// EPIC-044 US-001: Multiple similarity() with AND is now supported
// ============================================================================

#[test]
fn test_validate_multiple_similarity_with_and_passes() {
    // EPIC-044 US-001: Multiple similarity() with AND is now supported (cascade filtering)
    // Given: A query with multiple similarity() conditions using AND
    let query = create_query_with_multiple_similarity(); // Uses AND

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error - AND is allowed
    assert!(result.is_ok());
}

#[test]
fn test_validate_multiple_similarity_with_or_detected() {
    // Given: A query with multiple similarity() conditions using OR
    let query = create_query_with_multiple_similarity_or();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: ValidationError is returned - OR is not supported
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::MultipleSimilarity);
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
// EPIC-044 US-002: similarity() OR metadata is now supported (union mode)
// ============================================================================

#[test]
fn test_validate_or_with_similarity_now_passes() {
    // EPIC-044 US-002: similarity() OR metadata is NOW supported (union mode)
    // Given: A query with similarity() OR metadata
    let query = create_query_with_similarity_or_metadata();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error - union mode handles this
    assert!(result.is_ok());
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
fn test_validate_not_similarity_now_passes() {
    // EPIC-044 US-003: NOT similarity() is NOW supported via full scan
    // Given: A query with NOT similarity()
    let query = create_query_with_not_similarity();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error - NOT similarity() is now supported
    assert!(result.is_ok());
}

#[test]
fn test_validate_not_similarity_with_limit_passes() {
    // EPIC-044 US-003: NOT similarity() with LIMIT is supported
    // Given: A query with NOT similarity() and LIMIT
    let mut query = create_query_with_not_similarity();
    query.select.limit = Some(100);

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: No error
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
fn test_strict_mode_allows_not_similarity() {
    // EPIC-044 US-003: NOT similarity() is now supported
    // Given: A query with NOT similarity() without LIMIT
    let query = create_query_with_not_similarity();

    // When: Validation is performed with strict config
    let config = ValidationConfig::strict();
    let result = QueryValidator::validate_with_config(&query, &config);

    // Then: No error - NOT similarity() is supported via full scan
    assert!(result.is_ok());
}

// ============================================================================
// US-007: Error includes position information
// ============================================================================

#[test]
fn test_validation_error_kind_is_set() {
    // Given: A query with multiple similarity() using OR (should fail)
    let query = create_query_with_multiple_similarity_or();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: Error kind is correctly set
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::MultipleSimilarity);
    // Note: Position tracking not yet implemented (always None)
    // TODO: Implement position tracking in EPIC-044 US-008
    assert!(err.position.is_none());
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
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
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
        match_clause: None,
    }
}

/// Creates a query with multiple similarity() using OR (should fail validation).
fn create_query_with_multiple_similarity_or() -> Query {
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
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
            joins: vec![],
            where_clause: Some(Condition::Or(Box::new(sim1), Box::new(sim2))), // OR instead of AND
            order_by: None,
            limit: None,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: None,
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
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
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
        match_clause: None,
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
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
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
        match_clause: None,
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
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
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
        match_clause: None,
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
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
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
        match_clause: None,
    }
}

fn create_simple_query() -> Query {
    Query {
        select: SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
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
        match_clause: None,
    }
}

// ============================================================================
// Regression tests for PR #116 review bugs
// ============================================================================

#[test]
fn test_validate_vector_search_near_with_or_detected() {
    // BUG-001 regression: VectorSearch (NEAR) was not being validated
    // EPIC-044 US-001: Updated - multiple NEAR with OR should fail, AND should pass
    use crate::velesql::ast::VectorSearch;

    let near1 = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v1".to_string()),
    });
    let near2 = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v2".to_string()),
    });

    let query = Query {
        select: SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
            joins: vec![],
            where_clause: Some(Condition::Or(Box::new(near1), Box::new(near2))), // OR = invalid
            order_by: None,
            limit: None,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: None,
    };

    // Should detect multiple vector search with OR
    let result = QueryValidator::validate(&query);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::MultipleSimilarity);
}

#[test]
fn test_validate_vector_search_or_now_passes() {
    // EPIC-044 US-002: VectorSearch OR metadata is NOW supported (union mode)
    use crate::velesql::ast::VectorSearch;

    let near = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    });
    let meta = Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });

    let query = Query {
        select: SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
            joins: vec![],
            where_clause: Some(Condition::Or(Box::new(near), Box::new(meta))),
            order_by: None,
            limit: None,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: None,
    };

    // EPIC-044 US-002: NEAR OR metadata is now supported
    let result = QueryValidator::validate(&query);
    assert!(result.is_ok());
}

#[test]
fn test_validate_compound_query_where_clause() {
    // BUG-002 regression: Compound query's WHERE clause was not being validated
    // EPIC-044 US-001: Updated - multiple similarity with OR should fail, AND should pass
    use crate::velesql::ast::{CompoundQuery, SetOperator};

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

    // Main SELECT has no similarity
    // UNION's right side has multiple similarity with OR (invalid)
    let query = Query {
        select: SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
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
        compound: Some(CompoundQuery {
            operator: SetOperator::Union,
            right: Box::new(SelectStatement {
                distinct: crate::velesql::DistinctMode::None,
                columns: SelectColumns::All,
                from: "docs".to_string(),
                from_alias: None,
                joins: vec![],
                where_clause: Some(Condition::Or(Box::new(sim1), Box::new(sim2))), // OR = invalid
                order_by: None,
                limit: None,
                offset: None,
                with_clause: None,
                group_by: None,
                having: None,
                fusion_clause: None,
            }),
        }),
        match_clause: None,
    };

    // Should detect multiple similarity in OR in compound query
    let result = QueryValidator::validate(&query);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::MultipleSimilarity);
}
