//! Tests for ORDER BY clause execution and JSON value comparison.

#[cfg(test)]
mod tests {
    use crate::collection::search::query::ordering::{
        compare_json_values, evaluate_arithmetic, ScoreContext,
    };
    use crate::velesql::{ArithmeticExpr, ArithmeticOp, OrderByExpr};
    use serde_json::json;

    // -----------------------------------------------------------------------
    // compare_json_values — basic type comparisons
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_none_vs_none_is_equal() {
        assert_eq!(compare_json_values(None, None), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_compare_none_vs_some_is_less() {
        let v = json!(42);
        assert_eq!(
            compare_json_values(None, Some(&v)),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_some_vs_none_is_greater() {
        let v = json!("hello");
        assert_eq!(
            compare_json_values(Some(&v), None),
            std::cmp::Ordering::Greater
        );
    }

    // -----------------------------------------------------------------------
    // Same type: numbers
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_numbers_ascending() {
        let a = json!(10);
        let b = json!(20);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_numbers_equal() {
        let a = json!(42);
        let b = json!(42);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_compare_numbers_descending() {
        let a = json!(100);
        let b = json!(50);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Greater
        );
    }

    // -----------------------------------------------------------------------
    // Same type: strings
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_strings_alphabetical() {
        let a = json!("apple");
        let b = json!("banana");
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
    }

    // -----------------------------------------------------------------------
    // Same type: booleans
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_bools_false_less_than_true() {
        let a = json!(false);
        let b = json!(true);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less
        );
    }

    // -----------------------------------------------------------------------
    // Mixed types: type rank ordering
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_null_less_than_number() {
        let a = json!(null);
        let b = json!(42);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "null (rank 0) < number (rank 2)"
        );
    }

    #[test]
    fn test_compare_number_less_than_string() {
        let a = json!(99);
        let b = json!("99");
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "number (rank 2) < string (rank 3)"
        );
    }

    #[test]
    fn test_compare_string_less_than_array() {
        let a = json!("hello");
        let b = json!([1, 2, 3]);
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "string (rank 3) < array (rank 4)"
        );
    }

    #[test]
    fn test_compare_array_less_than_object() {
        let a = json!([1]);
        let b = json!({"key": "value"});
        assert_eq!(
            compare_json_values(Some(&a), Some(&b)),
            std::cmp::Ordering::Less,
            "array (rank 4) < object (rank 5)"
        );
    }

    // -----------------------------------------------------------------------
    // EPIC-042: Arithmetic expression evaluator
    // -----------------------------------------------------------------------

    #[test]
    fn test_arithmetic_eval_literal() {
        let expr = ArithmeticExpr::Literal(2.75);
        let ctx = ScoreContext::new(0.0, None);
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            (result - 2.75).abs() < 1e-5,
            "Literal should evaluate to its value"
        );
    }

    #[test]
    fn test_arithmetic_eval_variable_search_score() {
        let expr = ArithmeticExpr::Variable("vector_score".to_string());
        let ctx = ScoreContext::new(0.85, None);
        assert!((evaluate_arithmetic(&expr, &ctx) - 0.85).abs() < 1e-5);
    }

    #[test]
    fn test_arithmetic_eval_variable_from_payload() {
        let payload = json!({"boost_factor": 0.65});
        let expr = ArithmeticExpr::Variable("boost_factor".to_string());
        let ctx = ScoreContext::new(0.0, Some(&payload));
        assert!((evaluate_arithmetic(&expr, &ctx) - 0.65).abs() < 1e-5);
    }

    #[test]
    fn test_arithmetic_eval_missing_variable_returns_zero() {
        let payload = json!({"other": 10});
        let expr = ArithmeticExpr::Variable("missing_field".to_string());
        let ctx = ScoreContext::new(0.5, Some(&payload));
        assert!((evaluate_arithmetic(&expr, &ctx)).abs() < 1e-9);
    }

    #[test]
    fn test_arithmetic_eval_similarity_returns_search_score() {
        let expr = ArithmeticExpr::Similarity(Box::new(OrderByExpr::SimilarityBare));
        let ctx = ScoreContext::new(0.92, None);
        assert!((evaluate_arithmetic(&expr, &ctx) - 0.92).abs() < 1e-5);
    }

    #[test]
    fn test_arithmetic_ordering_weighted_scores() {
        // 0.7 * vector_score + 0.3 * boost_factor (payload field)
        let expr = ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Literal(0.7)),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Variable("vector_score".to_string())),
            }),
            op: ArithmeticOp::Add,
            right: Box::new(ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Literal(0.3)),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Variable("boost_factor".to_string())),
            }),
        };

        let payload = json!({"boost_factor": 0.8});
        let ctx = ScoreContext::new(0.9, Some(&payload));
        // Expected: 0.7 * 0.9 + 0.3 * 0.8 = 0.63 + 0.24 = 0.87
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            (result - 0.87).abs() < 1e-5,
            "Weighted score should be 0.87, got {result}"
        );
    }

    #[test]
    fn test_arithmetic_ordering_division_by_zero() {
        let expr = ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Literal(1.0)),
            op: ArithmeticOp::Div,
            right: Box::new(ArithmeticExpr::Literal(0.0)),
        };
        let ctx = ScoreContext::new(0.0, None);
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            result.abs() < 1e-9,
            "Division by zero should return 0.0, got {result}"
        );
    }

    #[test]
    fn test_arithmetic_eval_subtraction() {
        let expr = ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Variable("vector_score".to_string())),
            op: ArithmeticOp::Sub,
            right: Box::new(ArithmeticExpr::Literal(0.1)),
        };
        let ctx = ScoreContext::new(0.95, None);
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            (result - 0.85).abs() < 1e-5,
            "0.95 - 0.1 = 0.85, got {result}"
        );
    }

    #[test]
    fn test_arithmetic_eval_no_payload() {
        // When payload is None, all variable lookups return 0.0 except built-ins.
        let expr = ArithmeticExpr::Variable("custom_field".to_string());
        let ctx = ScoreContext::new(0.5, None);
        assert!(evaluate_arithmetic(&expr, &ctx).abs() < 1e-9);
    }

    // ------------------------------------------------------------------
    // Bug 3 regression: resolve_variable must recognize graph_score and bm25_score
    // ------------------------------------------------------------------

    #[test]
    fn test_resolve_variable_graph_score_returns_search_score() {
        let expr = ArithmeticExpr::Variable("graph_score".to_string());
        let ctx = ScoreContext::new(0.75, None);
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            (result - 0.75).abs() < 1e-5,
            "graph_score should resolve to search_score (0.75), got {result}"
        );
    }

    #[test]
    fn test_resolve_variable_bm25_score_returns_search_score() {
        let expr = ArithmeticExpr::Variable("bm25_score".to_string());
        let ctx = ScoreContext::new(0.60, None);
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            (result - 0.60).abs() < 1e-5,
            "bm25_score should resolve to search_score (0.60), got {result}"
        );
    }

    #[test]
    fn test_resolve_variable_bm25_score_not_shadowed_by_payload() {
        // Even when payload has a bm25_score field, the built-in should take
        // precedence to maintain consistent behavior.
        let payload = serde_json::json!({"bm25_score": 0.99});
        let expr = ArithmeticExpr::Variable("bm25_score".to_string());
        let ctx = ScoreContext::new(0.42, Some(&payload));
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            (result - 0.42).abs() < 1e-5,
            "bm25_score built-in should take precedence over payload (0.42), got {result}"
        );
    }

    // ------------------------------------------------------------------
    // Bug 1 regression: evaluate_arithmetic with Similarity(SimilarityBare)
    // should return search_score, and the bare form is the only supported form
    // ------------------------------------------------------------------

    #[test]
    fn test_arithmetic_eval_similarity_bare_inside_binary_op() {
        // 0.5 * similarity() + 0.5 * price
        // similarity() (bare) should resolve to ctx.search_score
        let expr = ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Literal(0.5)),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Similarity(Box::new(
                    OrderByExpr::SimilarityBare,
                ))),
            }),
            op: ArithmeticOp::Add,
            right: Box::new(ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Literal(0.5)),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Variable("price".to_string())),
            }),
        };

        let payload = serde_json::json!({"price": 0.8});
        let ctx = ScoreContext::new(0.9, Some(&payload));
        // Expected: 0.5 * 0.9 + 0.5 * 0.8 = 0.45 + 0.40 = 0.85
        let result = evaluate_arithmetic(&expr, &ctx);
        assert!(
            (result - 0.85).abs() < 1e-5,
            "0.5 * similarity() + 0.5 * price should be 0.85, got {result}"
        );
    }

    // -----------------------------------------------------------------------
    // Integration: ORDER BY field via VelesQL
    // -----------------------------------------------------------------------

    #[cfg(feature = "persistence")]
    mod integration {
        use crate::collection::types::Collection;
        use crate::distance::DistanceMetric;
        use crate::point::Point;
        use crate::velesql::Parser;
        use std::collections::HashMap;
        use std::path::PathBuf;

        fn setup_ordered_collection() -> (tempfile::TempDir, Collection) {
            let dir = tempfile::tempdir().expect("temp dir");
            let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
                .expect("create collection");

            let points = vec![
                Point {
                    id: 1,
                    vector: vec![1.0, 0.0, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 3, "name": "charlie"})),
                    sparse_vectors: None,
                },
                Point {
                    id: 2,
                    vector: vec![0.9, 0.1, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 1, "name": "alpha"})),
                    sparse_vectors: None,
                },
                Point {
                    id: 3,
                    vector: vec![0.8, 0.2, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 2, "name": "bravo"})),
                    sparse_vectors: None,
                },
            ];
            col.upsert(points).expect("upsert");
            (dir, col)
        }

        #[test]
        fn test_order_by_field_asc() {
            let (_dir, col) = setup_ordered_collection();

            let query = "SELECT * FROM test ORDER BY priority ASC LIMIT 10";
            let parsed = Parser::parse(query).expect("parse");
            let params = HashMap::new();

            let results = col.execute_query(&parsed, &params).expect("execute");
            assert!(results.len() >= 2, "should have results to sort");

            // Verify ascending order on priority.
            for window in results.windows(2) {
                let p0 = window[0]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("priority"))
                    .and_then(serde_json::Value::as_i64);
                let p1 = window[1]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("priority"))
                    .and_then(serde_json::Value::as_i64);
                assert!(
                    p0 <= p1,
                    "priority should be ascending: {:?} <= {:?}",
                    p0,
                    p1
                );
            }
        }

        #[test]
        fn test_order_by_field_desc() {
            let (_dir, col) = setup_ordered_collection();

            let query = "SELECT * FROM test ORDER BY name DESC LIMIT 10";
            let parsed = Parser::parse(query).expect("parse");
            let params = HashMap::new();

            let results = col.execute_query(&parsed, &params).expect("execute");
            assert!(results.len() >= 2, "should have results");

            // Verify descending order on name.
            for window in results.windows(2) {
                let n0 = window[0]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let n1 = window[1]
                    .point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                assert!(n0 >= n1, "name should be descending: {} >= {}", n0, n1);
            }
        }

        /// Regression test for bug #443: ORDER BY on a non-existent payload field
        /// must NOT drop results. The query should return the same number of
        /// results as the equivalent query without ORDER BY.
        #[test]
        fn test_order_by_nonexistent_field_preserves_results() {
            let (_dir, col) = setup_ordered_collection();
            let params = HashMap::new();

            // Baseline: no ORDER BY → expect 3 results.
            let baseline_query = "SELECT * FROM test LIMIT 10";
            let baseline_parsed = Parser::parse(baseline_query).expect("parse baseline");
            let baseline = col
                .execute_query(&baseline_parsed, &params)
                .expect("execute baseline");
            assert_eq!(baseline.len(), 3, "baseline should return all 3 points");

            // Bug query: ORDER BY a field that does not exist in any payload.
            let bug_query = "SELECT * FROM test ORDER BY nonexistent_field DESC LIMIT 10";
            let bug_parsed = Parser::parse(bug_query).expect("parse bug query");
            let results = col
                .execute_query(&bug_parsed, &params)
                .expect("execute bug query");

            assert_eq!(
                results.len(),
                baseline.len(),
                "ORDER BY on non-existent field must not drop results \
                 (got {} but expected {})",
                results.len(),
                baseline.len()
            );
        }

        /// Regression test for bug #443 with vector NEAR: ORDER BY a non-existent
        /// field after vector search must preserve the result count.
        #[test]
        fn test_order_by_nonexistent_field_with_near_preserves_results() {
            let (_dir, col) = setup_ordered_collection();
            let mut params = HashMap::new();
            params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

            // Baseline: NEAR without ORDER BY.
            let baseline_query = "SELECT * FROM test WHERE vector NEAR $v LIMIT 5";
            let baseline_parsed = Parser::parse(baseline_query).expect("parse baseline");
            let baseline = col
                .execute_query(&baseline_parsed, &params)
                .expect("execute baseline");
            assert!(
                !baseline.is_empty(),
                "NEAR query should return at least one result"
            );

            // Bug query: NEAR + ORDER BY non-existent field.
            let bug_query =
                "SELECT * FROM test WHERE vector NEAR $v ORDER BY fused_score DESC LIMIT 5";
            let bug_parsed = Parser::parse(bug_query).expect("parse bug query");
            let results = col
                .execute_query(&bug_parsed, &params)
                .expect("execute bug query");

            assert_eq!(
                results.len(),
                baseline.len(),
                "ORDER BY non-existent field after NEAR must not drop results \
                 (got {} but expected {})",
                results.len(),
                baseline.len()
            );
        }

        /// Regression test for bug #443 at scale: 50 points to exercise the
        /// HNSW index path (not just brute-force).
        #[test]
        fn test_order_by_nonexistent_field_at_scale_preserves_results() {
            let dir = tempfile::tempdir().expect("temp dir");
            let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
                .expect("create collection");

            // Insert 50 points so HNSW index is built.
            let points: Vec<crate::point::Point> = (0_u32..50)
                .map(|i| {
                    let f = f64::from(i) / 50.0;
                    #[allow(clippy::cast_possible_truncation)]
                    let v = vec![f as f32, (1.0 - f) as f32, 0.0, 0.0];
                    crate::point::Point {
                        id: u64::from(i),
                        vector: v,
                        payload: Some(serde_json::json!({
                            "category": format!("cat_{}", i % 5),
                            "priority": i % 10,
                        })),
                        sparse_vectors: None,
                    }
                })
                .collect();
            col.upsert(points).expect("upsert 50 points");

            let mut params = HashMap::new();
            params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.0, 0.0]));

            // Baseline: NEAR without ORDER BY.
            let baseline_query = "SELECT * FROM products WHERE vector NEAR $v LIMIT 10";
            let baseline_parsed = Parser::parse(baseline_query).expect("parse baseline");
            let baseline = col
                .execute_query(&baseline_parsed, &params)
                .expect("execute baseline");
            assert_eq!(baseline.len(), 10, "baseline should return 10 results");

            // Bug query: NEAR + ORDER BY non-existent field.
            let bug_query =
                "SELECT * FROM products WHERE vector NEAR $v ORDER BY fused_score DESC LIMIT 10";
            let bug_parsed = Parser::parse(bug_query).expect("parse bug query");
            let results = col
                .execute_query(&bug_parsed, &params)
                .expect("execute bug query");

            assert_eq!(
                results.len(),
                baseline.len(),
                "ORDER BY non-existent field after NEAR must not drop results \
                 (got {} but expected {})",
                results.len(),
                baseline.len()
            );
        }

        /// Bug #4 regression: ORDER BY a nested/dotted payload field must sort.
        /// Previously `compare_payload_field` did a flat lookup, so `meta.source`
        /// compared Equal for every row and the sort was a silent no-op.
        #[test]
        fn test_order_by_nested_payload_field_sorts() {
            let dir = tempfile::tempdir().expect("temp dir");
            let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
                .expect("create collection");

            let points = vec![
                Point {
                    id: 1,
                    vector: vec![1.0, 0.0, 0.0, 0.0],
                    payload: Some(serde_json::json!({"meta": {"source": "charlie"}})),
                    sparse_vectors: None,
                },
                Point {
                    id: 2,
                    vector: vec![0.9, 0.1, 0.0, 0.0],
                    payload: Some(serde_json::json!({"meta": {"source": "bravo"}})),
                    sparse_vectors: None,
                },
                Point {
                    id: 3,
                    vector: vec![0.8, 0.2, 0.0, 0.0],
                    payload: Some(serde_json::json!({"meta": {"source": "alpha"}})),
                    sparse_vectors: None,
                },
            ];
            col.upsert(points).expect("upsert");

            let query = "SELECT * FROM t ORDER BY meta.source ASC LIMIT 10";
            let parsed = Parser::parse(query).expect("parse");
            let params = HashMap::new();
            let results = col.execute_query(&parsed, &params).expect("execute");

            let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
            // alpha(id 3) < bravo(id 2) < charlie(id 1)
            assert_eq!(
                ids,
                vec![3, 2, 1],
                "ORDER BY nested payload meta.source ASC must sort, got {ids:?}"
            );
        }

        /// Bug #5 regression: ORDER BY similarity(named_field, $v) must score the
        /// NAMED payload vector, not the default vector. Multiple rows survive the
        /// WHERE filter so the ORDER BY of the survivors is what is under test; the
        /// `image_vec` ranking is deliberately the REVERSE of the default-vector
        /// ranking, so a default-vector sort would invert the expected order.
        #[test]
        fn test_order_by_similarity_named_vector_field() {
            let dir = tempfile::tempdir().expect("temp dir");
            let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
                .expect("create collection");

            // For $q = [1,0,0,0]:
            //   image_vec cosine: id1=0.6, id2=0.8, id3=1.0  (image ranking 3>2>1)
            //   default cosine:   id1=1.0, id2=0.8, id3=0.6  (default ranking 1>2>3)
            // All three clear the > 0.5 image_vec filter, so the survivor set is
            // {1,2,3} and only the ORDER BY decides the sequence.
            let points = vec![
                Point {
                    id: 1,
                    vector: vec![1.0, 0.0, 0.0, 0.0],
                    payload: Some(serde_json::json!({"image_vec": [0.6, 0.8, 0.0, 0.0]})),
                    sparse_vectors: None,
                },
                Point {
                    id: 2,
                    vector: vec![0.8, 0.6, 0.0, 0.0],
                    payload: Some(serde_json::json!({"image_vec": [0.8, 0.6, 0.0, 0.0]})),
                    sparse_vectors: None,
                },
                Point {
                    id: 3,
                    vector: vec![0.6, 0.8, 0.0, 0.0],
                    payload: Some(serde_json::json!({"image_vec": [1.0, 0.0, 0.0, 0.0]})),
                    sparse_vectors: None,
                },
            ];
            col.upsert(points).expect("upsert");

            let mut params = HashMap::new();
            params.insert("q".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

            let query = "SELECT * FROM t WHERE similarity(image_vec, $q) > 0.5 \
                         ORDER BY similarity(image_vec, $q) DESC LIMIT 10";
            let parsed = Parser::parse(query).expect("parse");
            let results = col.execute_query(&parsed, &params).expect("execute");
            let by_image: Vec<u64> = results.iter().map(|r| r.point.id).collect();

            // image_vec similarity DESC => 3 (1.0) > 2 (0.8) > 1 (0.6).
            // A default-vector sort would yield 1 > 2 > 3 (the reverse).
            assert_eq!(
                by_image,
                vec![3, 2, 1],
                "ORDER BY similarity(image_vec,$q) must rank by the NAMED vector \
                 (3,2,1), got {by_image:?}"
            );
        }

        /// EPIC-042: Integration test for arithmetic ORDER BY with execute_query.
        /// Verifies that `ORDER BY 0.7 * similarity() + 0.3 * priority DESC`
        /// returns results sorted by the weighted combination.
        #[test]
        fn test_order_by_arithmetic_weighted_with_near() {
            let dir = tempfile::tempdir().expect("temp dir");
            let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
                .expect("create collection");

            let points = vec![
                Point {
                    id: 1,
                    vector: vec![1.0, 0.0, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 0.1})),
                    sparse_vectors: None,
                },
                Point {
                    id: 2,
                    vector: vec![0.9, 0.1, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 0.9})),
                    sparse_vectors: None,
                },
                Point {
                    id: 3,
                    vector: vec![0.5, 0.5, 0.0, 0.0],
                    payload: Some(serde_json::json!({"priority": 0.5})),
                    sparse_vectors: None,
                },
            ];
            col.upsert(points).expect("upsert");

            let mut params = HashMap::new();
            params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

            // Use arithmetic ORDER BY with weighted scoring.
            let query = "SELECT * FROM test WHERE vector NEAR $v \
                         ORDER BY 0.5 * similarity() + 0.5 * priority DESC LIMIT 10";
            let parsed = Parser::parse(query).expect("parse");
            let results = col.execute_query(&parsed, &params).expect("execute");

            assert_eq!(
                results.len(),
                3,
                "all 3 upserted points should be returned (LIMIT 10)"
            );
            let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
            // Weighted score = 0.5*similarity() + 0.5*priority, DESC:
            //   point 2: 0.5*0.994 + 0.5*0.9 = 0.947  (wins despite not being the nearest)
            //   point 3: 0.5*0.707 + 0.5*0.5 = 0.604
            //   point 1: 0.5*1.000 + 0.5*0.1 = 0.550
            assert_eq!(
                ids,
                vec![2, 3, 1],
                "results must be sorted by weighted score DESC, got {ids:?}"
            );
            assert_eq!(
                results[0].point.id, 2,
                "highest weighted score (point 2) must rank first under DESC"
            );
        }
    }
}
