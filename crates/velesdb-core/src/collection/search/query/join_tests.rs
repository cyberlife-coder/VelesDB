//! Tests for `join` module - JOIN execution for VelesQL.

use super::join::{adaptive_batch_size, execute_join, extract_join_keys, joined_to_search_results};
use crate::column_store::{ColumnStore, ColumnType, ColumnValue};
use crate::point::Point;
use crate::point::SearchResult;
use crate::velesql::{ColumnRef, JoinClause, JoinCondition, JoinType};

fn make_search_result(id: u64, payload_id: i64) -> SearchResult {
    SearchResult {
        point: Point {
            id,
            vector: vec![0.1, 0.2, 0.3],
            payload: Some(serde_json::json!({"id": payload_id, "name": format!("item_{}", id)})),
        },
        score: 0.9,
    }
}

fn make_column_store() -> ColumnStore {
    let mut store = ColumnStore::with_primary_key(
        &[
            ("product_id", ColumnType::Int),
            ("price", ColumnType::Float),
            ("available", ColumnType::Bool),
        ],
        "product_id",
    )
    .unwrap();

    store
        .insert_row(&[
            ("product_id", ColumnValue::Int(1)),
            ("price", ColumnValue::Float(99.99)),
            ("available", ColumnValue::Bool(true)),
        ])
        .unwrap();
    store
        .insert_row(&[
            ("product_id", ColumnValue::Int(2)),
            ("price", ColumnValue::Float(149.99)),
            ("available", ColumnValue::Bool(false)),
        ])
        .unwrap();
    store
        .insert_row(&[
            ("product_id", ColumnValue::Int(3)),
            ("price", ColumnValue::Float(49.99)),
            ("available", ColumnValue::Bool(true)),
        ])
        .unwrap();

    store
}

fn make_join_clause() -> JoinClause {
    JoinClause {
        join_type: crate::velesql::JoinType::Inner,
        table: "prices".to_string(),
        alias: None,
        condition: Some(JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        }),
        using_columns: None,
    }
}

#[test]
fn test_adaptive_batch_size_small() {
    assert_eq!(adaptive_batch_size(50), 50);
    assert_eq!(adaptive_batch_size(100), 100);
}

#[test]
fn test_adaptive_batch_size_medium() {
    assert_eq!(adaptive_batch_size(101), 1000);
    assert_eq!(adaptive_batch_size(5000), 1000);
    assert_eq!(adaptive_batch_size(10000), 1000);
}

#[test]
fn test_adaptive_batch_size_large() {
    assert_eq!(adaptive_batch_size(10001), 5000);
    assert_eq!(adaptive_batch_size(100_000), 5000);
}

#[test]
fn test_extract_join_keys() {
    let results = vec![
        make_search_result(1, 1),
        make_search_result(2, 2),
        make_search_result(3, 3),
    ];
    let join = make_join_clause();

    let keys = extract_join_keys(&results, join.condition.as_ref().unwrap());

    assert_eq!(keys.len(), 3);
    assert_eq!(keys[0], (0, 1));
    assert_eq!(keys[1], (1, 2));
    assert_eq!(keys[2], (2, 3));
}

#[test]
fn test_execute_join_basic() {
    let results = vec![
        make_search_result(1, 1),
        make_search_result(2, 2),
        make_search_result(3, 3),
    ];
    let column_store = make_column_store();
    let join = make_join_clause();

    let joined = execute_join(&results, &join, &column_store).expect("execute_join failed");

    assert_eq!(joined.len(), 3);
    assert!(joined[0].column_data.contains_key("price"));
    let price = joined[0]
        .column_data
        .get("price")
        .unwrap()
        .as_f64()
        .unwrap();
    assert!((price - 99.99).abs() < 0.01);
}

#[test]
fn test_execute_join_inner_skips_missing() {
    let results = vec![
        make_search_result(1, 1),
        make_search_result(2, 99),
        make_search_result(3, 3),
    ];
    let column_store = make_column_store();
    let join = make_join_clause();

    let joined = execute_join(&results, &join, &column_store).expect("execute_join failed");
    assert_eq!(joined.len(), 2);
}

#[test]
fn test_joined_to_search_results() {
    let results = vec![make_search_result(1, 1)];
    let column_store = make_column_store();
    let join = make_join_clause();

    let joined = execute_join(&results, &join, &column_store).expect("execute_join failed");
    let search_results = joined_to_search_results(joined);

    assert_eq!(search_results.len(), 1);
    let payload = search_results[0].point.payload.as_ref().unwrap();
    assert!(payload.get("price").is_some());
    assert!(payload.get("available").is_some());
}

// ========== REGRESSION TESTS FOR PR #85 BUGS ==========

#[test]
fn test_extract_join_keys_u64_overflow_safety() {
    let large_id = u64::MAX;
    let result = SearchResult {
        point: Point {
            id: large_id,
            vector: vec![0.1, 0.2, 0.3],
            payload: None,
        },
        score: 0.9,
    };

    let condition = JoinCondition {
        left: ColumnRef {
            table: Some("prices".to_string()),
            column: "product_id".to_string(),
        },
        right: ColumnRef {
            table: Some("products".to_string()),
            column: "id".to_string(),
        },
    };

    let keys = extract_join_keys(&[result], &condition);

    assert!(
        keys.is_empty() || keys.iter().all(|(_, k)| *k >= 0),
        "Large u64 IDs should not produce negative join keys: {:?}",
        keys
    );
}

#[test]
fn test_extract_join_keys_i64_max_boundary() {
    let max_safe_id = i64::MAX as u64;
    let result = SearchResult {
        point: Point {
            id: max_safe_id,
            vector: vec![0.1, 0.2, 0.3],
            payload: None,
        },
        score: 0.9,
    };

    let condition = JoinCondition {
        left: ColumnRef {
            table: Some("prices".to_string()),
            column: "product_id".to_string(),
        },
        right: ColumnRef {
            table: Some("products".to_string()),
            column: "id".to_string(),
        },
    };

    let keys = extract_join_keys(&[result], &condition);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].1, i64::MAX);
}

#[test]
fn test_extract_join_keys_just_above_i64_max() {
    let just_over = (i64::MAX as u64) + 1;
    let result = SearchResult {
        point: Point {
            id: just_over,
            vector: vec![0.1, 0.2, 0.3],
            payload: None,
        },
        score: 0.9,
    };

    let condition = JoinCondition {
        left: ColumnRef {
            table: Some("prices".to_string()),
            column: "product_id".to_string(),
        },
        right: ColumnRef {
            table: Some("products".to_string()),
            column: "id".to_string(),
        },
    };

    let keys = extract_join_keys(&[result], &condition);
    assert!(
        keys.is_empty(),
        "IDs > i64::MAX should be filtered out, got: {:?}",
        keys
    );
}

#[test]
fn test_execute_join_validates_pk_column() {
    let results = vec![make_search_result(1, 1)];
    let column_store = make_column_store();

    let wrong_join = JoinClause {
        join_type: crate::velesql::JoinType::Inner,
        table: "prices".to_string(),
        alias: None,
        condition: Some(JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "category_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        }),
        using_columns: None,
    };

    let joined = execute_join(&results, &wrong_join, &column_store).expect("execute_join failed");
    assert!(
        joined.is_empty(),
        "JOIN on non-PK column should not return results silently"
    );
}

#[test]
fn test_execute_join_correct_pk_column_works() {
    let results = vec![make_search_result(1, 1)];
    let column_store = make_column_store();

    let correct_join = JoinClause {
        join_type: crate::velesql::JoinType::Inner,
        table: "prices".to_string(),
        alias: None,
        condition: Some(JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        }),
        using_columns: None,
    };

    let joined = execute_join(&results, &correct_join, &column_store).expect("execute_join failed");
    assert_eq!(joined.len(), 1);
}

// ========== LEFT JOIN TESTS (Phase 08-02) ==========

fn make_left_join_clause() -> JoinClause {
    JoinClause {
        join_type: JoinType::Left,
        table: "prices".to_string(),
        alias: None,
        condition: Some(JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        }),
        using_columns: None,
    }
}

#[test]
fn test_left_join_keeps_all_left_rows() {
    // Result id=99 has no match in column store (only has PKs 1,2,3)
    let results = vec![
        make_search_result(1, 1),
        make_search_result(2, 99), // No match
        make_search_result(3, 3),
    ];
    let column_store = make_column_store();
    let join = make_left_join_clause();

    let joined = execute_join(&results, &join, &column_store).expect("execute_join failed");

    // LEFT JOIN: all 3 rows should be returned
    assert_eq!(
        joined.len(),
        3,
        "LEFT JOIN should keep all left rows, got {}",
        joined.len()
    );
}

#[test]
fn test_left_join_merges_matching_rows() {
    let results = vec![
        make_search_result(1, 1),
        make_search_result(2, 99), // No match
        make_search_result(3, 3),
    ];
    let column_store = make_column_store();
    let join = make_left_join_clause();

    let joined = execute_join(&results, &join, &column_store).expect("execute_join failed");

    // Matching rows should have price data
    assert!(
        joined[0].column_data.contains_key("price"),
        "Matching row should have price"
    );

    // Non-matching row should have empty column_data
    assert!(
        joined[1].column_data.is_empty(),
        "Non-matching row should have empty column data"
    );

    // Third row matches
    assert!(
        joined[2].column_data.contains_key("price"),
        "Matching row should have price"
    );
}

#[test]
fn test_left_join_converted_to_search_results() {
    let results = vec![
        make_search_result(1, 1),
        make_search_result(2, 99), // No match
    ];
    let column_store = make_column_store();
    let join = make_left_join_clause();

    let joined = execute_join(&results, &join, &column_store).expect("execute_join failed");
    let search_results = joined_to_search_results(joined);

    assert_eq!(search_results.len(), 2);

    // Matching row has price in payload
    let payload_1 = search_results[0].point.payload.as_ref().unwrap();
    assert!(payload_1.get("price").is_some());

    // Non-matching row still has original payload
    let payload_2 = search_results[1].point.payload.as_ref().unwrap();
    assert!(payload_2.get("name").is_some());
}

#[test]
fn test_inner_join_still_filters_non_matching() {
    // Verify INNER JOIN behavior unchanged after LEFT JOIN addition
    let results = vec![
        make_search_result(1, 1),
        make_search_result(2, 99), // No match
        make_search_result(3, 3),
    ];
    let column_store = make_column_store();
    let join = make_join_clause(); // INNER JOIN

    let joined = execute_join(&results, &join, &column_store).expect("execute_join failed");

    // INNER JOIN: only 2 matching rows
    assert_eq!(
        joined.len(),
        2,
        "INNER JOIN should only return matching rows"
    );
}

// ========== RIGHT/FULL JOIN ERROR TESTS (Phase 08-02 completion) ==========

#[test]
fn test_right_join_returns_error() {
    let results = vec![make_search_result(1, 1)];
    let column_store = make_column_store();

    let right_join = JoinClause {
        join_type: JoinType::Right,
        table: "prices".to_string(),
        alias: None,
        condition: Some(JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        }),
        using_columns: None,
    };

    let result = execute_join(&results, &right_join, &column_store);
    assert!(result.is_err(), "RIGHT JOIN should return error");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("RIGHT JOIN"),
        "Error should mention RIGHT JOIN: {}",
        err
    );
}

#[test]
fn test_full_join_returns_error() {
    let results = vec![make_search_result(1, 1)];
    let column_store = make_column_store();

    let full_join = JoinClause {
        join_type: JoinType::Full,
        table: "prices".to_string(),
        alias: None,
        condition: Some(JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "product_id".to_string(),
            },
            right: ColumnRef {
                table: Some("products".to_string()),
                column: "id".to_string(),
            },
        }),
        using_columns: None,
    };

    let result = execute_join(&results, &full_join, &column_store);
    assert!(result.is_err(), "FULL JOIN should return error");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("FULL JOIN"),
        "Error should mention FULL JOIN: {}",
        err
    );
}
