//! Tests for similarity() function parsing in VelesQL.
//!
//! TDD: These tests are written BEFORE implementation.

#[cfg(test)]
mod tests {
    use crate::velesql::ast::{CompareOp, Condition, VectorExpr};
    use crate::velesql::Parser;

    // ============================================
    // BASIC PARSING TESTS
    // ============================================

    #[test]
    fn test_similarity_with_parameter_greater_than() {
        let query = "SELECT * FROM docs WHERE similarity(embedding, $query_vec) > 0.8";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

        let stmt = result.unwrap();
        if let Some(ref condition) = stmt.select.where_clause {
            match condition {
                Condition::Similarity(sim) => {
                    assert_eq!(sim.field, "embedding");
                    assert!(
                        matches!(sim.vector, VectorExpr::Parameter(ref name) if name == "query_vec")
                    );
                    assert_eq!(sim.operator, CompareOp::Gt);
                    assert!((sim.threshold - 0.8).abs() < 0.001);
                }
                _ => panic!("Expected Similarity condition, got {:?}", condition),
            }
        } else {
            panic!("Expected condition in statement");
        }
    }

    #[test]
    fn test_similarity_with_literal_vector() {
        let query = "SELECT * FROM docs WHERE similarity(embedding, [0.1, 0.2, 0.3]) >= 0.5";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

        let stmt = result.unwrap();
        if let Some(ref condition) = stmt.select.where_clause {
            match condition {
                Condition::Similarity(sim) => {
                    assert_eq!(sim.field, "embedding");
                    if let VectorExpr::Literal(vec) = &sim.vector {
                        assert_eq!(vec.len(), 3);
                        assert!((vec[0] - 0.1).abs() < 0.001);
                    } else {
                        panic!("Expected literal vector");
                    }
                    assert_eq!(sim.operator, CompareOp::Gte);
                    assert!((sim.threshold - 0.5).abs() < 0.001);
                }
                _ => panic!("Expected Similarity condition"),
            }
        } else {
            panic!("Expected condition in statement");
        }
    }

    #[test]
    fn test_similarity_less_than() {
        let query = "SELECT * FROM docs WHERE similarity(vec_field, $v) < 0.3";
        let result = Parser::parse(query);
        assert!(result.is_ok());

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert_eq!(sim.operator, CompareOp::Lt);
            assert!((sim.threshold - 0.3).abs() < 0.001);
        } else {
            panic!("Expected Similarity condition");
        }
    }

    #[test]
    fn test_similarity_less_than_or_equal() {
        let query = "SELECT * FROM docs WHERE similarity(vec, $v) <= 0.9";
        let result = Parser::parse(query);
        assert!(result.is_ok());

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert_eq!(sim.operator, CompareOp::Lte);
        } else {
            panic!("Expected Similarity condition");
        }
    }

    #[test]
    fn test_similarity_equal() {
        let query = "SELECT * FROM docs WHERE similarity(emb, $q) = 1.0";
        let result = Parser::parse(query);
        assert!(result.is_ok());

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert_eq!(sim.operator, CompareOp::Eq);
            assert!((sim.threshold - 1.0).abs() < 0.001);
        } else {
            panic!("Expected Similarity condition");
        }
    }

    // ============================================
    // COMBINED CONDITIONS TESTS
    // ============================================

    #[test]
    fn test_similarity_with_and_condition() {
        let query =
            "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.7 AND category = 'tech'";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

        let stmt = result.unwrap();
        if let Some(Condition::And(left, _right)) = &stmt.select.where_clause {
            assert!(matches!(left.as_ref(), Condition::Similarity(_)));
        } else {
            panic!("Expected AND condition");
        }
    }

    #[test]
    fn test_similarity_with_or_condition() {
        let query =
            "SELECT * FROM docs WHERE similarity(emb1, $v1) > 0.8 OR similarity(emb2, $v2) > 0.8";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

        let stmt = result.unwrap();
        if let Some(Condition::Or(left, right)) = &stmt.select.where_clause {
            assert!(matches!(left.as_ref(), Condition::Similarity(_)));
            assert!(matches!(right.as_ref(), Condition::Similarity(_)));
        } else {
            panic!("Expected OR condition");
        }
    }

    // ============================================
    // EDGE CASES AND ERROR HANDLING
    // ============================================

    #[test]
    fn test_similarity_zero_threshold() {
        let query = "SELECT * FROM docs WHERE similarity(emb, $v) > 0.0";
        let result = Parser::parse(query);
        assert!(result.is_ok());

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert!((sim.threshold - 0.0).abs() < 0.001);
        } else {
            panic!("Expected Similarity condition");
        }
    }

    #[test]
    fn test_similarity_one_threshold() {
        let query = "SELECT * FROM docs WHERE similarity(emb, $v) >= 1.0";
        let result = Parser::parse(query);
        assert!(result.is_ok());
    }

    #[test]
    fn test_similarity_negative_threshold_parsed() {
        // Grammar supports negative floats via: float = @{ "-"? ~ ASCII_DIGIT+ ~ "." ~ ASCII_DIGIT+ }
        // Parser should accept negative thresholds; semantic validation (if needed) happens later
        let query = "SELECT * FROM docs WHERE similarity(emb, $v) > -0.5";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "Parser should accept negative threshold values"
        );
    }

    #[test]
    fn test_similarity_missing_field_error() {
        let query = "SELECT * FROM docs WHERE similarity(, $v) > 0.5";
        let result = Parser::parse(query);
        assert!(result.is_err());
    }

    #[test]
    fn test_similarity_missing_vector_error() {
        let query = "SELECT * FROM docs WHERE similarity(emb, ) > 0.5";
        let result = Parser::parse(query);
        assert!(result.is_err());
    }

    #[test]
    fn test_similarity_missing_threshold_error() {
        let query = "SELECT * FROM docs WHERE similarity(emb, $v) >";
        let result = Parser::parse(query);
        assert!(result.is_err());
    }

    #[test]
    fn test_similarity_missing_operator_error() {
        let query = "SELECT * FROM docs WHERE similarity(emb, $v) 0.5";
        let result = Parser::parse(query);
        assert!(result.is_err());
    }

    // ============================================
    // FIELD NAME VARIATIONS
    // ============================================

    #[test]
    fn test_similarity_dotted_field_name() {
        let query = "SELECT * FROM docs WHERE similarity(node.embedding, $v) > 0.8";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "Failed to parse dotted field: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert_eq!(sim.field, "node.embedding");
        } else {
            panic!("Expected Similarity condition");
        }
    }

    #[test]
    fn test_similarity_underscore_field_name() {
        let query = "SELECT * FROM docs WHERE similarity(my_embedding_field, $v) > 0.5";
        let result = Parser::parse(query);
        assert!(result.is_ok());

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert_eq!(sim.field, "my_embedding_field");
        } else {
            panic!("Expected Similarity condition");
        }
    }

    /// Test that integer thresholds are accepted (not just floats).
    /// This reduces user confusion when writing queries like `> 1` instead of `> 1.0`.
    #[test]
    fn test_similarity_integer_threshold() {
        let query = "SELECT * FROM docs WHERE similarity(embedding, $v) > 1";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "Integer threshold should parse: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert_eq!(sim.operator, CompareOp::Gt);
            assert!((sim.threshold - 1.0).abs() < 0.001);
        } else {
            panic!("Expected Similarity condition");
        }
    }

    #[test]
    fn test_similarity_negative_integer_threshold() {
        let query = "SELECT * FROM docs WHERE similarity(embedding, $v) >= -1";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "Negative integer threshold should parse: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        if let Some(Condition::Similarity(sim)) = &stmt.select.where_clause {
            assert!((sim.threshold - (-1.0)).abs() < 0.001);
        } else {
            panic!("Expected Similarity condition");
        }
    }

    // ============================================
    // ORDER BY SIMILARITY TESTS (EPIC-008 US-008)
    // ============================================

    #[test]
    fn test_order_by_similarity_desc() {
        let query =
            "SELECT * FROM docs WHERE category = 'tech' ORDER BY similarity(embedding, $v) DESC LIMIT 10";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "ORDER BY similarity DESC should parse: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        assert!(stmt.select.order_by.is_some());
        let order_by = stmt.select.order_by.as_ref().unwrap();
        assert_eq!(order_by.len(), 1);
        assert!(order_by[0].descending);
        // Check it's a similarity expression
        match &order_by[0].expr {
            crate::velesql::OrderByExpr::Similarity(sim) => {
                assert_eq!(sim.field, "embedding");
                assert!(matches!(sim.vector, VectorExpr::Parameter(ref name) if name == "v"));
            }
            crate::velesql::OrderByExpr::Field(_) | crate::velesql::OrderByExpr::Aggregate(_) => {
                panic!("Expected OrderByExpr::Similarity")
            }
        }
    }

    #[test]
    fn test_order_by_similarity_asc() {
        let query = "SELECT * FROM docs ORDER BY similarity(embedding, $v) ASC LIMIT 5";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "ORDER BY similarity ASC should parse: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        assert!(stmt.select.order_by.is_some());
        let order_by = stmt.select.order_by.as_ref().unwrap();
        assert!(!order_by[0].descending);
    }

    #[test]
    fn test_order_by_similarity_default_desc() {
        // Default for similarity should be DESC (highest similarity first)
        let query = "SELECT * FROM docs ORDER BY similarity(embedding, $v) LIMIT 10";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "ORDER BY similarity without direction should parse: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        assert!(stmt.select.order_by.is_some());
        let order_by = stmt.select.order_by.as_ref().unwrap();
        // Default should be DESC for similarity
        assert!(order_by[0].descending);
    }

    #[test]
    fn test_order_by_field() {
        let query = "SELECT * FROM docs ORDER BY created_at DESC LIMIT 10";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "ORDER BY field should parse: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        assert!(stmt.select.order_by.is_some());
        let order_by = stmt.select.order_by.as_ref().unwrap();
        match &order_by[0].expr {
            crate::velesql::OrderByExpr::Field(name) => {
                assert_eq!(name, "created_at");
            }
            crate::velesql::OrderByExpr::Similarity(_)
            | crate::velesql::OrderByExpr::Aggregate(_) => panic!("Expected OrderByExpr::Field"),
        }
    }

    #[test]
    fn test_order_by_multiple() {
        let query =
            "SELECT * FROM docs ORDER BY similarity(embedding, $v) DESC, created_at ASC LIMIT 10";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "Multiple ORDER BY should parse: {:?}",
            result.err()
        );

        let stmt = result.unwrap();
        assert!(stmt.select.order_by.is_some());
        let order_by = stmt.select.order_by.as_ref().unwrap();
        assert_eq!(order_by.len(), 2);
        assert!(order_by[0].descending);
        assert!(!order_by[1].descending);
    }

    /// BUG-001: compute_similarity must respect collection's configured metric
    /// This test verifies that ORDER BY similarity() uses the collection's metric
    /// instead of always using cosine similarity.
    ///
    /// Semantics: DESC = most similar first, ASC = least similar first
    #[test]
    #[cfg(feature = "persistence")]
    fn test_order_by_similarity_respects_collection_metric() {
        use crate::distance::DistanceMetric;
        use crate::Collection;

        let temp_dir = tempfile::tempdir().unwrap();
        let path = std::path::PathBuf::from(temp_dir.path());

        // Create collection with EUCLIDEAN metric (lower distance = more similar)
        let collection = Collection::create(path, 3, DistanceMetric::Euclidean).unwrap();

        // Insert test vectors
        // v1 = [1, 0, 0], v2 = [0, 1, 0], v3 = [0.5, 0.5, 0]
        collection
            .upsert(vec![
                crate::point::Point {
                    id: 1,
                    vector: vec![1.0, 0.0, 0.0],
                    payload: Some(serde_json::json!({"name": "v1"})),
                },
                crate::point::Point {
                    id: 2,
                    vector: vec![0.0, 1.0, 0.0],
                    payload: Some(serde_json::json!({"name": "v2"})),
                },
                crate::point::Point {
                    id: 3,
                    vector: vec![0.5, 0.5, 0.0],
                    payload: Some(serde_json::json!({"name": "v3"})),
                },
            ])
            .unwrap();

        // Query vector: [1, 0, 0]
        // Euclidean distances: v1=0.0 (closest), v3≈0.707, v2≈1.414 (farthest)
        //
        // ASC = least similar first → v2 (farthest), v3, v1 (closest)
        // This verifies the metric is correctly used (Euclidean, not hardcoded cosine)

        let query = "SELECT * FROM test ORDER BY similarity(vector, $v) ASC LIMIT 3";
        let parsed = crate::velesql::Parser::parse(query).unwrap();
        let mut params = std::collections::HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // ASC = least similar first, so v2 should be first (distance ~1.414)
        assert_eq!(results.len(), 3);
        assert_eq!(
            results[0].point.id, 2,
            "Euclidean ASC should return v2 first (least similar, distance ~1.414)"
        );
        assert_eq!(
            results[2].point.id, 1,
            "Euclidean ASC should return v1 last (most similar, distance 0)"
        );
    }

    /// Test ORDER BY similarity DESC with Euclidean metric.
    /// DESC should always mean "most similar first" regardless of metric type.
    #[test]
    #[cfg(feature = "persistence")]
    fn test_order_by_similarity_desc_euclidean_metric() {
        use crate::distance::DistanceMetric;
        use crate::Collection;

        let temp_dir = tempfile::tempdir().unwrap();
        let path = std::path::PathBuf::from(temp_dir.path());

        // Create collection with EUCLIDEAN metric (lower = more similar)
        let collection = Collection::create(path, 3, DistanceMetric::Euclidean).unwrap();

        // Insert test vectors
        collection
            .upsert(vec![
                crate::point::Point {
                    id: 1,
                    vector: vec![1.0, 0.0, 0.0],
                    payload: Some(serde_json::json!({"name": "v1"})),
                },
                crate::point::Point {
                    id: 2,
                    vector: vec![0.0, 1.0, 0.0],
                    payload: Some(serde_json::json!({"name": "v2"})),
                },
                crate::point::Point {
                    id: 3,
                    vector: vec![0.5, 0.5, 0.0],
                    payload: Some(serde_json::json!({"name": "v3"})),
                },
            ])
            .unwrap();

        // Query vector: [1, 0, 0]
        // Euclidean distances: v1=0.0, v3≈0.707, v2≈1.414
        // DESC should return: v1 (most similar), v3, v2 (least similar)
        let query = "SELECT * FROM test ORDER BY similarity(vector, $v) DESC LIMIT 3";
        let parsed = crate::velesql::Parser::parse(query).unwrap();
        let mut params = std::collections::HashMap::new();
        params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0]));

        let results = collection.execute_query(&parsed, &params).unwrap();

        // DESC = most similar first, so v1 should be first (distance 0)
        assert_eq!(results.len(), 3);
        assert_eq!(
            results[0].point.id, 1,
            "Euclidean DESC should return v1 first (most similar, distance 0)"
        );
        assert_eq!(
            results[2].point.id, 2,
            "Euclidean DESC should return v2 last (least similar, distance ~1.414)"
        );
    }
}
