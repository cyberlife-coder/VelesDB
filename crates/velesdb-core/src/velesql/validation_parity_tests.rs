//! Parity tests for validation modules (EPIC-065).
//!
//! These tests ensure that the public `QueryValidator` and internal Collection
//! validation methods produce consistent results.
#![cfg(all(test, feature = "persistence"))]

mod tests {
    use crate::velesql::ast::{
        CompareOp, Comparison, Condition, SelectColumns, SelectStatement, SimilarityCondition,
        Value, VectorExpr, VectorFusedSearch, VectorSearch,
    };
    use crate::velesql::{Query, QueryValidator};

    fn make_query(where_clause: Option<Condition>) -> Query {
        Query {
            select: SelectStatement {
                distinct: crate::velesql::DistinctMode::None,
                columns: SelectColumns::All,
                from: "test".to_string(),
                from_alias: None,
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
            match_clause: None,
        }
    }

    fn similarity_condition() -> Condition {
        Condition::Similarity(SimilarityCondition {
            field: "embedding".to_string(),
            vector: VectorExpr::Parameter("v".to_string()),
            operator: CompareOp::Gt,
            threshold: 0.8,
        })
    }

    fn vector_search_condition() -> Condition {
        Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Parameter("v".to_string()),
        })
    }

    fn vector_fused_condition() -> Condition {
        Condition::VectorFusedSearch(VectorFusedSearch {
            vectors: vec![VectorExpr::Parameter("v1".to_string())],
            fusion: crate::velesql::FusionConfig::default(),
        })
    }

    fn metadata_condition() -> Condition {
        Condition::Comparison(Comparison {
            column: "category".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("tech".to_string()),
        })
    }

    // =========================================================================
    // PARITY TESTS: Ensure public and internal validation behave identically
    // =========================================================================

    #[test]
    fn test_parity_single_similarity_allowed() {
        let condition = similarity_condition();
        let query = make_query(Some(condition.clone()));

        // Public API should pass
        assert!(QueryValidator::validate(&query).is_ok());

        // Internal validation should pass
        assert!(
            crate::collection::Collection::validate_similarity_query_structure(&condition).is_ok()
        );
    }

    #[test]
    fn test_parity_single_vector_search_allowed() {
        let condition = vector_search_condition();
        let query = make_query(Some(condition.clone()));

        assert!(QueryValidator::validate(&query).is_ok());
        assert!(
            crate::collection::Collection::validate_similarity_query_structure(&condition).is_ok()
        );
    }

    #[test]
    fn test_parity_single_fused_search_allowed() {
        let condition = vector_fused_condition();
        let query = make_query(Some(condition.clone()));

        assert!(QueryValidator::validate(&query).is_ok());
        assert!(
            crate::collection::Collection::validate_similarity_query_structure(&condition).is_ok()
        );
    }

    #[test]
    fn test_parity_multiple_similarity_with_and_allowed() {
        // EPIC-044 US-001: Multiple similarity() with AND is allowed
        let condition = Condition::And(
            Box::new(similarity_condition()),
            Box::new(Condition::Similarity(SimilarityCondition {
                field: "embedding".to_string(),
                vector: VectorExpr::Parameter("v2".to_string()),
                operator: CompareOp::Gt,
                threshold: 0.7,
            })),
        );
        let query = make_query(Some(condition.clone()));

        assert!(QueryValidator::validate(&query).is_ok());
        assert!(
            crate::collection::Collection::validate_similarity_query_structure(&condition).is_ok()
        );
    }

    #[test]
    fn test_parity_multiple_similarity_with_or_rejected() {
        // Multiple similarity() in OR is not supported (would require union)
        let condition = Condition::Or(
            Box::new(similarity_condition()),
            Box::new(Condition::Similarity(SimilarityCondition {
                field: "embedding".to_string(),
                vector: VectorExpr::Parameter("v2".to_string()),
                operator: CompareOp::Gt,
                threshold: 0.7,
            })),
        );
        let query = make_query(Some(condition.clone()));

        // Both should reject
        assert!(QueryValidator::validate(&query).is_err());
        assert!(
            crate::collection::Collection::validate_similarity_query_structure(&condition).is_err()
        );
    }

    #[test]
    fn test_parity_similarity_or_metadata_allowed() {
        // EPIC-044 US-002: similarity() OR metadata IS now supported
        let condition = Condition::Or(
            Box::new(similarity_condition()),
            Box::new(metadata_condition()),
        );
        let query = make_query(Some(condition.clone()));

        // Both should allow (union mode)
        assert!(QueryValidator::validate(&query).is_ok());
        assert!(
            crate::collection::Collection::validate_similarity_query_structure(&condition).is_ok()
        );
    }

    #[test]
    fn test_parity_all_vector_types_counted() {
        // Ensure all vector search types are counted equally
        let conditions = vec![
            similarity_condition(),
            vector_search_condition(),
            vector_fused_condition(),
        ];

        for cond in conditions {
            // Create OR with metadata - should pass (single vector search)
            let or_with_meta =
                Condition::Or(Box::new(cond.clone()), Box::new(metadata_condition()));
            let query = make_query(Some(or_with_meta.clone()));

            assert!(
                QueryValidator::validate(&query).is_ok(),
                "Single vector search OR metadata should be allowed"
            );
            assert!(
                crate::collection::Collection::validate_similarity_query_structure(&or_with_meta)
                    .is_ok(),
                "Single vector search OR metadata should be allowed"
            );
        }
    }
}
