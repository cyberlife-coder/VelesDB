//! Tests for window functions (Issue #386 Phase 1).
//!
//! Covers: grammar parsing, AST construction, evaluator logic,
//! edge cases (ties, empty partitions, NULL values, no PARTITION BY).

#[cfg(test)]
mod tests {
    use crate::point::{Point, SearchResult};
    use crate::velesql::{
        window_evaluator, OverClause, Parser, SelectColumns, WindowFunction, WindowFunctionType,
        WindowOrderBy,
    };

    // ================================================================
    // Helper functions
    // ================================================================

    fn make_result(id: u64, payload: serde_json::Value, score: f32) -> SearchResult {
        SearchResult::new(
            Point {
                id,
                vector: vec![0.0; 4],
                payload: Some(payload),
                sparse_vectors: None,
            },
            score,
        )
    }

    fn get_payload_u64(result: &SearchResult, field: &str) -> Option<u64> {
        result
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get(field))
            .and_then(serde_json::Value::as_u64)
    }

    // ================================================================
    // Parser tests — grammar → AST
    // ================================================================

    #[test]
    fn test_parse_row_number_with_partition_and_order() {
        let query = Parser::parse(
            "SELECT name, ROW_NUMBER() OVER (PARTITION BY category ORDER BY price ASC) AS rn FROM products",
        )
        .unwrap();

        match &query.select.columns {
            SelectColumns::Mixed {
                columns,
                window_functions,
                ..
            } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "name");

                assert_eq!(window_functions.len(), 1);
                let wf = &window_functions[0];
                assert_eq!(wf.function_type, WindowFunctionType::RowNumber);
                assert_eq!(wf.alias, Some("rn".to_string()));

                assert_eq!(wf.over_clause.partition_by, vec!["category".to_string()]);
                assert_eq!(wf.over_clause.order_by.len(), 1);
                assert_eq!(wf.over_clause.order_by[0].column, "price");
                assert!(!wf.over_clause.order_by[0].descending);
            }
            other => panic!("Expected Mixed, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_rank_desc() {
        let query =
            Parser::parse("SELECT RANK() OVER (ORDER BY score DESC) AS rnk FROM docs").unwrap();

        match &query.select.columns {
            SelectColumns::Mixed {
                window_functions, ..
            } => {
                assert_eq!(window_functions.len(), 1);
                let wf = &window_functions[0];
                assert_eq!(wf.function_type, WindowFunctionType::Rank);
                assert_eq!(wf.alias, Some("rnk".to_string()));
                assert!(wf.over_clause.partition_by.is_empty());
                assert_eq!(wf.over_clause.order_by[0].column, "score");
                assert!(wf.over_clause.order_by[0].descending);
            }
            other => panic!("Expected Mixed, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_dense_rank_no_alias() {
        let query = Parser::parse(
            "SELECT DENSE_RANK() OVER (PARTITION BY dept ORDER BY salary DESC) FROM employees",
        )
        .unwrap();

        match &query.select.columns {
            SelectColumns::Mixed {
                window_functions, ..
            } => {
                assert_eq!(window_functions.len(), 1);
                let wf = &window_functions[0];
                assert_eq!(wf.function_type, WindowFunctionType::DenseRank);
                assert!(wf.alias.is_none());
                assert_eq!(wf.over_clause.partition_by, vec!["dept".to_string()]);
            }
            other => panic!("Expected Mixed, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_multiple_partition_columns() {
        let query = Parser::parse(
            "SELECT ROW_NUMBER() OVER (PARTITION BY region, department ORDER BY hire_date ASC) AS rn FROM emp",
        )
        .unwrap();

        match &query.select.columns {
            SelectColumns::Mixed {
                window_functions, ..
            } => {
                let wf = &window_functions[0];
                assert_eq!(
                    wf.over_clause.partition_by,
                    vec!["region".to_string(), "department".to_string()]
                );
            }
            other => panic!("Expected Mixed, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_window_with_similarity_order() {
        let query = Parser::parse(
            "SELECT ROW_NUMBER() OVER (PARTITION BY source ORDER BY similarity() DESC) AS rn FROM docs",
        )
        .unwrap();

        match &query.select.columns {
            SelectColumns::Mixed {
                window_functions, ..
            } => {
                let wf = &window_functions[0];
                assert_eq!(wf.over_clause.order_by[0].column, "similarity");
                assert!(wf.over_clause.order_by[0].descending);
            }
            other => panic!("Expected Mixed, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_rank_as_column_name_no_ambiguity() {
        // "rank" without parens should parse as a regular column, not a window function
        let query = Parser::parse("SELECT rank FROM docs").unwrap();
        match &query.select.columns {
            SelectColumns::Columns(cols) => {
                assert_eq!(cols.len(), 1);
                assert_eq!(cols[0].name, "rank");
            }
            other => panic!("Expected Columns, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_multiple_order_by_in_over() {
        let query = Parser::parse(
            "SELECT ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC, name ASC) AS rn FROM emp",
        )
        .unwrap();

        match &query.select.columns {
            SelectColumns::Mixed {
                window_functions, ..
            } => {
                let wf = &window_functions[0];
                assert_eq!(wf.over_clause.order_by.len(), 2);
                assert_eq!(wf.over_clause.order_by[0].column, "salary");
                assert!(wf.over_clause.order_by[0].descending);
                assert_eq!(wf.over_clause.order_by[1].column, "name");
                assert!(!wf.over_clause.order_by[1].descending);
            }
            other => panic!("Expected Mixed, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_case_insensitive_keywords() {
        // Verify case-insensitive parsing for all window keywords
        let query = Parser::parse(
            "SELECT row_number() over (partition by cat order by val desc) AS rn FROM t",
        )
        .unwrap();

        match &query.select.columns {
            SelectColumns::Mixed {
                window_functions, ..
            } => {
                assert_eq!(
                    window_functions[0].function_type,
                    WindowFunctionType::RowNumber
                );
            }
            other => panic!("Expected Mixed, got: {:?}", other),
        }
    }

    // ================================================================
    // Evaluator tests — window function computation
    // ================================================================

    #[test]
    fn test_row_number_single_partition() {
        let mut results = vec![
            make_result(1, serde_json::json!({"name": "C", "score": 30}), 0.3),
            make_result(2, serde_json::json!({"name": "A", "score": 10}), 0.1),
            make_result(3, serde_json::json!({"name": "B", "score": 20}), 0.2),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // Sorted by score ASC: A(10)=1, B(20)=2, C(30)=3
        assert_eq!(get_payload_u64(&results[1], "rn"), Some(1)); // A (id=2)
        assert_eq!(get_payload_u64(&results[2], "rn"), Some(2)); // B (id=3)
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(3)); // C (id=1)
    }

    #[test]
    fn test_row_number_multiple_partitions() {
        let mut results = vec![
            make_result(1, serde_json::json!({"cat": "A", "val": 10}), 0.1),
            make_result(2, serde_json::json!({"cat": "B", "val": 20}), 0.2),
            make_result(3, serde_json::json!({"cat": "A", "val": 30}), 0.3),
            make_result(4, serde_json::json!({"cat": "B", "val": 40}), 0.4),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec!["cat".to_string()],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // Partition A: id=1(val=10)→1, id=3(val=30)→2
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1));
        assert_eq!(get_payload_u64(&results[2], "rn"), Some(2));

        // Partition B: id=2(val=20)→1, id=4(val=40)→2
        assert_eq!(get_payload_u64(&results[1], "rn"), Some(1));
        assert_eq!(get_payload_u64(&results[3], "rn"), Some(2));
    }

    #[test]
    fn test_rank_with_ties() {
        let mut results = vec![
            make_result(1, serde_json::json!({"score": 100}), 1.0),
            make_result(2, serde_json::json!({"score": 90}), 0.9),
            make_result(3, serde_json::json!({"score": 90}), 0.9),
            make_result(4, serde_json::json!({"score": 80}), 0.8),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::Rank,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: true,
                }],
            },
            alias: Some("rnk".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // 100→1, 90→2, 90→2, 80→4 (gap after tie)
        assert_eq!(get_payload_u64(&results[0], "rnk"), Some(1)); // 100
        assert_eq!(get_payload_u64(&results[1], "rnk"), Some(2)); // 90
        assert_eq!(get_payload_u64(&results[2], "rnk"), Some(2)); // 90
        assert_eq!(get_payload_u64(&results[3], "rnk"), Some(4)); // 80
    }

    #[test]
    fn test_dense_rank_with_ties() {
        let mut results = vec![
            make_result(1, serde_json::json!({"score": 100}), 1.0),
            make_result(2, serde_json::json!({"score": 90}), 0.9),
            make_result(3, serde_json::json!({"score": 90}), 0.9),
            make_result(4, serde_json::json!({"score": 80}), 0.8),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::DenseRank,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: true,
                }],
            },
            alias: Some("drnk".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // 100→1, 90→2, 90→2, 80→3 (no gap)
        assert_eq!(get_payload_u64(&results[0], "drnk"), Some(1));
        assert_eq!(get_payload_u64(&results[1], "drnk"), Some(2));
        assert_eq!(get_payload_u64(&results[2], "drnk"), Some(2));
        assert_eq!(get_payload_u64(&results[3], "drnk"), Some(3));
    }

    #[test]
    fn test_empty_result_set() {
        let mut results: Vec<SearchResult> = vec![];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        // Should succeed with no crash
        window_evaluator::evaluate(&mut results, &[wf]).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_single_result() {
        let mut results = vec![make_result(1, serde_json::json!({"val": 42}), 0.5)];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1));
    }

    #[test]
    fn test_null_partition_values() {
        let mut results = vec![
            make_result(1, serde_json::json!({"cat": "A", "val": 1}), 0.1),
            make_result(2, serde_json::json!({"val": 2}), 0.2), // No "cat" field → null partition
            make_result(3, serde_json::json!({"cat": "A", "val": 3}), 0.3),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec!["cat".to_string()],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // Partition A: id=1(val=1)→1, id=3(val=3)→2
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1));
        assert_eq!(get_payload_u64(&results[2], "rn"), Some(2));

        // Null partition: id=2→1
        assert_eq!(get_payload_u64(&results[1], "rn"), Some(1));
    }

    #[test]
    fn test_default_alias() {
        let mut results = vec![make_result(1, serde_json::json!({"val": 1}), 0.5)];

        // No alias → uses function_type.default_alias()
        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: None,
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();
        assert_eq!(get_payload_u64(&results[0], "row_number"), Some(1));
    }

    #[test]
    fn test_sort_by_similarity_score() {
        let mut results = vec![
            make_result(1, serde_json::json!({"source": "web"}), 0.95),
            make_result(2, serde_json::json!({"source": "web"}), 0.80),
            make_result(3, serde_json::json!({"source": "web"}), 0.90),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec!["source".to_string()],
                order_by: vec![WindowOrderBy {
                    column: "similarity".to_string(),
                    descending: true,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // Sorted by similarity DESC: 0.95→1, 0.90→2, 0.80→3
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1)); // score=0.95
        assert_eq!(get_payload_u64(&results[2], "rn"), Some(2)); // score=0.90
        assert_eq!(get_payload_u64(&results[1], "rn"), Some(3)); // score=0.80
    }

    #[test]
    fn test_all_same_values_rank() {
        let mut results = vec![
            make_result(1, serde_json::json!({"score": 50}), 0.5),
            make_result(2, serde_json::json!({"score": 50}), 0.5),
            make_result(3, serde_json::json!({"score": 50}), 0.5),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::Rank,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rnk".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // All tied → all rank 1
        for r in &results {
            assert_eq!(get_payload_u64(r, "rnk"), Some(1));
        }
    }

    #[test]
    fn test_all_same_values_dense_rank() {
        let mut results = vec![
            make_result(1, serde_json::json!({"score": 50}), 0.5),
            make_result(2, serde_json::json!({"score": 50}), 0.5),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::DenseRank,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: false,
                }],
            },
            alias: Some("drnk".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // All tied → all dense_rank 1
        for r in &results {
            assert_eq!(get_payload_u64(r, "drnk"), Some(1));
        }
    }

    #[test]
    fn test_window_function_type_default_alias() {
        assert_eq!(WindowFunctionType::RowNumber.default_alias(), "row_number");
        assert_eq!(WindowFunctionType::Rank.default_alias(), "rank");
        assert_eq!(WindowFunctionType::DenseRank.default_alias(), "dense_rank");
    }

    #[test]
    fn test_nested_partition_column() {
        let mut results = vec![
            make_result(
                1,
                serde_json::json!({"metadata": {"source": "web"}, "val": 10}),
                0.1,
            ),
            make_result(
                2,
                serde_json::json!({"metadata": {"source": "api"}, "val": 20}),
                0.2,
            ),
            make_result(
                3,
                serde_json::json!({"metadata": {"source": "web"}, "val": 30}),
                0.3,
            ),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec!["metadata.source".to_string()],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // Web partition: id=1(val=10)→1, id=3(val=30)→2
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1));
        assert_eq!(get_payload_u64(&results[2], "rn"), Some(2));

        // API partition: id=2→1
        assert_eq!(get_payload_u64(&results[1], "rn"), Some(1));
    }

    #[test]
    fn test_desc_order_row_number() {
        let mut results = vec![
            make_result(1, serde_json::json!({"val": 10}), 0.1),
            make_result(2, serde_json::json!({"val": 30}), 0.3),
            make_result(3, serde_json::json!({"val": 20}), 0.2),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: true,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).unwrap();

        // DESC: val=30→1, val=20→2, val=10→3
        assert_eq!(get_payload_u64(&results[1], "rn"), Some(1)); // val=30 (id=2)
        assert_eq!(get_payload_u64(&results[2], "rn"), Some(2)); // val=20 (id=3)
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(3)); // val=10 (id=1)
    }

    // =====================================================================
    // Regression tests — the three 🔴 critical bugs flagged by Devin on
    // the original #629 and fixed before merge.
    // =====================================================================

    /// Bug #2 regression: `extract_sort_value` used to special-case the
    /// column name `"score"` and return `result.score` (the search
    /// similarity score) instead of the user's payload field. Any payload
    /// with its own `score` column would silently order by the search
    /// score instead of the column value — hidden until a user's payload
    /// score distribution diverges from the similarity ranking.
    ///
    /// This test constructs that exact mismatch: similarity scores are in
    /// `[0.1, 0.3, 0.5, 0.2]` order but payload scores are
    /// `[100, 50, 75, 200]`. Ordering by payload `score` DESC must yield
    /// id 4 (200) first, not id 3 (similarity 0.5) first.
    #[test]
    fn test_order_by_payload_score_is_not_hijacked_by_similarity() {
        let mut results = vec![
            make_result(1, serde_json::json!({"score": 100}), 0.1),
            make_result(2, serde_json::json!({"score": 50}), 0.3),
            make_result(3, serde_json::json!({"score": 75}), 0.5),
            make_result(4, serde_json::json!({"score": 200}), 0.2),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: true,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).expect("evaluate");

        // Expected order by payload.score DESC: 200, 100, 75, 50 → ids 4, 1, 3, 2.
        assert_eq!(
            get_payload_u64(&results[3], "rn"),
            Some(1),
            "id 4 payload=200 should rank 1"
        );
        assert_eq!(
            get_payload_u64(&results[0], "rn"),
            Some(2),
            "id 1 payload=100 should rank 2"
        );
        assert_eq!(
            get_payload_u64(&results[2], "rn"),
            Some(3),
            "id 3 payload=75 should rank 3"
        );
        assert_eq!(
            get_payload_u64(&results[1], "rn"),
            Some(4),
            "id 2 payload=50 should rank 4"
        );
    }

    /// Bug #3 regression: `RANK() OVER (ORDER BY score DESC) AS score` used
    /// to corrupt its own input. The old loop wrote the rank value back
    /// into `payload["score"]` after each iteration; the next iteration's
    /// tie-detection read this corrupted value instead of the original
    /// sort key. For input `[100, 90, 90, 80]` it produced `[1, 2, 3, 4]`
    /// instead of the SQL-correct `[1, 2, 2, 4]`.
    #[test]
    fn test_rank_alias_collides_with_order_by_column_preserves_ties() {
        let mut results = vec![
            make_result(1, serde_json::json!({"score": 100}), 0.5),
            make_result(2, serde_json::json!({"score": 90}), 0.5),
            make_result(3, serde_json::json!({"score": 90}), 0.5),
            make_result(4, serde_json::json!({"score": 80}), 0.5),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::Rank,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: true,
                }],
            },
            // Alias deliberately collides with the ORDER BY column.
            alias: Some("score".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).expect("evaluate");

        // SQL-correct RANK() for [100, 90, 90, 80] DESC is [1, 2, 2, 4].
        // The pre-fix implementation produced [1, 2, 3, 4] because the
        // "score" alias overwrote the sort key used by tie detection.
        assert_eq!(get_payload_u64(&results[0], "score"), Some(1), "100 → 1");
        assert_eq!(
            get_payload_u64(&results[1], "score"),
            Some(2),
            "90 → 2 (tie)"
        );
        assert_eq!(
            get_payload_u64(&results[2], "score"),
            Some(2),
            "90 → 2 (tie)"
        );
        assert_eq!(
            get_payload_u64(&results[3], "score"),
            Some(4),
            "80 → 4 (gap after ties)"
        );
    }

    /// Bug #4 regression: **inter-function contamination via ORDER BY**.
    ///
    /// Before the fix, `evaluate` snapshotted ORDER BY values inside
    /// `apply_single_window`, so each function's snapshot was taken **after**
    /// all earlier functions had already injected their rank values into the
    /// payload. Two window functions with a colliding alias/column pair
    /// would corrupt each other's inputs.
    ///
    /// Scenario: `ROW_NUMBER() OVER (ORDER BY score DESC) AS score,
    /// RANK()       OVER (ORDER BY score DESC) AS rnk` with payload scores
    /// `[100, 90, 90, 80]` and similarity `0.5` for every row.
    ///
    /// - ROW_NUMBER ranks by original payload `score` DESC → `[1, 2, 3, 4]`
    ///   and injects those values into `payload["score"]`.
    /// - RANK must ALSO see the original `[100, 90, 90, 80]` and produce
    ///   `[1, 2, 2, 4]`. The pre-fix code read the corrupted
    ///   `payload["score"]` values (now `1,2,3,4`, all distinct) and
    ///   produced `[1, 2, 3, 4]` with no tie detection.
    #[test]
    fn test_inter_function_contamination_via_order_by_column() {
        let mut results = vec![
            make_result(1, serde_json::json!({"score": 100}), 0.5),
            make_result(2, serde_json::json!({"score": 90}), 0.5),
            make_result(3, serde_json::json!({"score": 90}), 0.5),
            make_result(4, serde_json::json!({"score": 80}), 0.5),
        ];

        // First window function: ROW_NUMBER aliased to "score" (collides with
        // the payload column AND with the second function's ORDER BY key).
        let wf_row_number = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: true,
                }],
            },
            alias: Some("score".to_string()),
        };

        // Second window function: RANK, ORDER BY "score" — which the first
        // function has just overwritten. If snapshots are not taken up-front,
        // this reads the injected row-numbers instead of the original scores.
        let wf_rank = WindowFunction {
            function_type: WindowFunctionType::Rank,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "score".to_string(),
                    descending: true,
                }],
            },
            alias: Some("rnk".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf_row_number, wf_rank]).expect("evaluate");

        // ROW_NUMBER on original [100, 90, 90, 80] DESC → [1, 2, 3, 4]
        // (stable order: ids 1, 2, 3, 4 respectively). The alias "score"
        // overwrites the payload.
        assert_eq!(get_payload_u64(&results[0], "score"), Some(1));
        assert_eq!(get_payload_u64(&results[1], "score"), Some(2));
        assert_eq!(get_payload_u64(&results[2], "score"), Some(3));
        assert_eq!(get_payload_u64(&results[3], "score"), Some(4));

        // RANK on the ORIGINAL [100, 90, 90, 80] DESC must be [1, 2, 2, 4].
        // Pre-fix, RANK saw [1, 2, 3, 4] (all distinct) and produced
        // [1, 2, 3, 4] — no tie was detected, breaking SQL semantics.
        assert_eq!(
            get_payload_u64(&results[0], "rnk"),
            Some(1),
            "id 1 (original score=100) must rank 1"
        );
        assert_eq!(
            get_payload_u64(&results[1], "rnk"),
            Some(2),
            "id 2 (original score=90) must rank 2 (tie)"
        );
        assert_eq!(
            get_payload_u64(&results[2], "rnk"),
            Some(2),
            "id 3 (original score=90) must rank 2 (tie)"
        );
        assert_eq!(
            get_payload_u64(&results[3], "rnk"),
            Some(4),
            "id 4 (original score=80) must rank 4 (gap after ties)"
        );
    }

    /// Bug #4 regression: **inter-function contamination via PARTITION BY**.
    ///
    /// Same contamination mechanism as the ORDER BY variant above, but via
    /// a partition key. The first window function writes its rank into a
    /// column that the second function uses as its `PARTITION BY`.
    ///
    /// Scenario: four rows split across two original `group` values
    /// (`"A"` and `"B"`), each row also has its own `val`.
    /// - First WF: `ROW_NUMBER() OVER (ORDER BY val) AS group` — overwrites
    ///   `payload["group"]` with `1..4` (all distinct, so every row would
    ///   become its own partition if the second function reads the injected
    ///   value).
    /// - Second WF: `RANK() OVER (PARTITION BY group ORDER BY val) AS rnk`
    ///   must still see the ORIGINAL `"A"` / `"B"` grouping.
    ///
    /// With the fix, `RANK` sees 2 partitions (2 rows each). Pre-fix, it
    /// would see 4 partitions of 1 row each and every row would rank 1.
    #[test]
    fn test_inter_function_contamination_via_partition_by_column() {
        let mut results = vec![
            make_result(1, serde_json::json!({"group": "A", "val": 10}), 0.5),
            make_result(2, serde_json::json!({"group": "B", "val": 20}), 0.5),
            make_result(3, serde_json::json!({"group": "A", "val": 30}), 0.5),
            make_result(4, serde_json::json!({"group": "B", "val": 40}), 0.5),
        ];

        let wf_row_number = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            // Alias collides with the second function's PARTITION BY column.
            alias: Some("group".to_string()),
        };

        let wf_rank = WindowFunction {
            function_type: WindowFunctionType::Rank,
            over_clause: OverClause {
                partition_by: vec!["group".to_string()],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rnk".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf_row_number, wf_rank]).expect("evaluate");

        // ROW_NUMBER over val ASC → 10→1, 20→2, 30→3, 40→4.
        assert_eq!(get_payload_u64(&results[0], "group"), Some(1));
        assert_eq!(get_payload_u64(&results[1], "group"), Some(2));
        assert_eq!(get_payload_u64(&results[2], "group"), Some(3));
        assert_eq!(get_payload_u64(&results[3], "group"), Some(4));

        // RANK inside ORIGINAL partition "A" (ids 1, 3) by val ASC → 1, 2.
        // RANK inside ORIGINAL partition "B" (ids 2, 4) by val ASC → 1, 2.
        // Pre-fix, each row would be its own partition (because the "group"
        // column now holds unique values 1..4) and every rnk would be 1.
        assert_eq!(
            get_payload_u64(&results[0], "rnk"),
            Some(1),
            "id 1 in partition A, val=10 → rank 1"
        );
        assert_eq!(
            get_payload_u64(&results[2], "rnk"),
            Some(2),
            "id 3 in partition A, val=30 → rank 2"
        );
        assert_eq!(
            get_payload_u64(&results[1], "rnk"),
            Some(1),
            "id 2 in partition B, val=20 → rank 1"
        );
        assert_eq!(
            get_payload_u64(&results[3], "rnk"),
            Some(2),
            "id 4 in partition B, val=40 → rank 2"
        );
    }

    // =====================================================================
    // Zero-tech-debt pass: regressions for the 🚩 informational findings
    // that were resolved in the hardening commit.
    // =====================================================================

    /// Partition-key collision regression: typed JSON values that render to
    /// the same bytes via naive `to_string()` must still produce distinct
    /// partition keys.
    ///
    /// Before the fix, `extract_payload_value` returned the inner string for
    /// `Value::String(s)` (stripping the JSON quotes) and `"__null__"` for
    /// `Value::Null`. Consequences:
    /// - `Value::Number(1)` and `Value::String("1")` both rendered as `"1"`
    ///   and collided into the same partition.
    /// - `Value::Null` and a literal payload string `"__null__"` both rendered
    ///   as `"__null__"` and collided into the same partition.
    ///
    /// The fix uses `serde_json`'s canonical `Display` (`Value::to_string`),
    /// which preserves the JSON type discriminator (bare `1`, quoted `"1"`,
    /// literal `null`, quoted `"null"`).
    #[test]
    fn test_partition_key_does_not_collide_int_and_string_of_same_digits() {
        // Two rows with the SAME `val` but DIFFERENT payload types for the
        // partition column `"cat"`. Before the fix they'd end up in the same
        // partition and rank 1, 2. After the fix they're in separate
        // partitions and both rank 1.
        let mut results = vec![
            make_result(1, serde_json::json!({"cat": 1, "val": 10}), 0.5),
            make_result(2, serde_json::json!({"cat": "1", "val": 20}), 0.5),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec!["cat".to_string()],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).expect("evaluate");

        // Each row is the sole member of its own partition → both rank 1.
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1), "cat=int(1)");
        assert_eq!(
            get_payload_u64(&results[1], "rn"),
            Some(1),
            "cat=str(\"1\")"
        );
    }

    /// Partition-key NULL-sentinel regression: a payload string literally
    /// equal to `"__null__"` must not collide with a missing/Null field.
    #[test]
    fn test_partition_key_distinguishes_null_from_literal_sentinel() {
        let mut results = vec![
            // Row 1: actual JSON null for "cat".
            make_result(1, serde_json::json!({"cat": null, "val": 10}), 0.5),
            // Row 2: literal string "__null__" for "cat".
            make_result(2, serde_json::json!({"cat": "__null__", "val": 20}), 0.5),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec!["cat".to_string()],
                order_by: vec![WindowOrderBy {
                    column: "val".to_string(),
                    descending: false,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).expect("evaluate");

        // Distinct partitions → both rank 1.
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1), "cat=null");
        assert_eq!(
            get_payload_u64(&results[1], "rn"),
            Some(1),
            "cat=\"__null__\""
        );
    }

    /// Design-intent regression: DISTINCT runs BEFORE window functions.
    ///
    /// VelesQL deliberately deviates from the SQL-standard logical order
    /// (which is `SELECT (windows) → DISTINCT`) so that `ROW_NUMBER` /
    /// `RANK` over a DISTINCT set produces a dense `1..N` without gaps,
    /// matching the "top-N distinct titles ranked by similarity" vector
    /// search pattern. This evaluator contract drives that behaviour: when
    /// evaluate is called, DISTINCT has already reduced the row set.
    ///
    /// This test verifies the *evaluator's* invariant: given a row set that
    /// has already been deduped (as the pipeline would hand it over), the
    /// window function numbers the survivors contiguously. If the pipeline
    /// order were ever flipped, this test would still pass (because it
    /// feeds a pre-deduped slice directly) — its purpose is to pin the
    /// evaluator semantics, while the pipeline order contract is pinned by
    /// the doc comment on `apply_select_postprocessing`.
    #[test]
    fn test_row_number_numbers_deduped_rows_contiguously() {
        // Simulate what the pipeline hands to the evaluator after DISTINCT:
        // three survivors with unique titles, sorted by similarity.
        let mut results = vec![
            make_result(10, serde_json::json!({"title": "A"}), 0.9),
            make_result(20, serde_json::json!({"title": "B"}), 0.7),
            make_result(30, serde_json::json!({"title": "C"}), 0.5),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::RowNumber,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "similarity".to_string(),
                    descending: true,
                }],
            },
            alias: Some("rn".to_string()),
        };

        window_evaluator::evaluate(&mut results, &[wf]).expect("evaluate");

        // Contiguous 1, 2, 3 — no gaps, matching vector-search expectation.
        assert_eq!(get_payload_u64(&results[0], "rn"), Some(1));
        assert_eq!(get_payload_u64(&results[1], "rn"), Some(2));
        assert_eq!(get_payload_u64(&results[2], "rn"), Some(3));
    }

    /// Bug #3 regression with the *default* alias. `RANK()` without an
    /// explicit `AS` uses `WindowFunctionType::default_alias() == "rank"`,
    /// and a user who happens to have a `rank` payload field and sorts by
    /// it would hit the same corruption path.
    #[test]
    fn test_rank_default_alias_collides_with_payload_field_preserves_ties() {
        let mut results = vec![
            make_result(1, serde_json::json!({"rank": 10}), 0.5),
            make_result(2, serde_json::json!({"rank": 5}), 0.5),
            make_result(3, serde_json::json!({"rank": 5}), 0.5),
            make_result(4, serde_json::json!({"rank": 1}), 0.5),
        ];

        let wf = WindowFunction {
            function_type: WindowFunctionType::Rank,
            over_clause: OverClause {
                partition_by: vec![],
                order_by: vec![WindowOrderBy {
                    column: "rank".to_string(),
                    descending: true,
                }],
            },
            // No explicit alias → default_alias() is "rank", collides.
            alias: None,
        };

        window_evaluator::evaluate(&mut results, &[wf]).expect("evaluate");

        // Same pattern as the explicit-alias test: [10, 5, 5, 1] → [1, 2, 2, 4].
        assert_eq!(get_payload_u64(&results[0], "rank"), Some(1));
        assert_eq!(get_payload_u64(&results[1], "rank"), Some(2));
        assert_eq!(get_payload_u64(&results[2], "rank"), Some(2));
        assert_eq!(get_payload_u64(&results[3], "rank"), Some(4));
    }
}
