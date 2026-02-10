//! Tests for scalar subquery execution (VP-002).
//!
//! Verifies that `execute_scalar_subquery` correctly executes inner SELECT
//! statements and returns scalar values for use in WHERE clause comparisons.

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;

use crate::velesql::{self, Condition, Subquery, Value};
use crate::{Database, DistanceMetric, Point};

/// Helper: create a collection with products at various prices for subquery testing.
fn setup_subquery_test_collection() -> (TempDir, crate::Collection) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");

    db.create_collection("products", 4, DistanceMetric::Cosine)
        .expect("Failed to create collection");
    let collection = db
        .get_collection("products")
        .expect("Failed to get collection");

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "name": "Widget A",
                "price": 10,
                "category": "electronics"
            })),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "name": "Widget B",
                "price": 20,
                "category": "electronics"
            })),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "name": "Widget C",
                "price": 30,
                "category": "clothing"
            })),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({
                "name": "Widget D",
                "price": 40,
                "category": "electronics"
            })),
        ),
        Point::new(
            5,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({
                "name": "Widget E",
                "price": 50,
                "category": "clothing"
            })),
        ),
    ];

    collection.upsert(points).expect("Failed to upsert points");
    (temp_dir, collection)
}

/// Helper: build a Subquery AST node for `SELECT AGG(col) FROM collection`.
fn make_aggregate_subquery(
    agg_type: velesql::AggregateType,
    column: &str,
    from: &str,
    where_clause: Option<Condition>,
) -> Subquery {
    use velesql::{AggregateArg, AggregateFunction, DistinctMode, SelectColumns, SelectStatement};

    let agg = AggregateFunction {
        function_type: agg_type,
        argument: AggregateArg::Column(column.to_string()),
        alias: None,
    };

    let select = SelectStatement {
        distinct: DistinctMode::None,
        columns: SelectColumns::Aggregations(vec![agg]),
        from: from.to_string(),
        from_alias: None,
        joins: Vec::new(),
        where_clause,
        order_by: None,
        limit: None,
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    Subquery {
        select,
        correlations: Vec::new(),
    }
}

/// Helper: build a Subquery AST node for `SELECT col FROM collection WHERE ... LIMIT 1`.
fn make_scalar_select_subquery(
    column: &str,
    from: &str,
    where_clause: Option<Condition>,
) -> Subquery {
    use velesql::{Column, DistinctMode, SelectColumns, SelectStatement};

    let select = SelectStatement {
        distinct: DistinctMode::None,
        columns: SelectColumns::Columns(vec![Column::new(column)]),
        from: from.to_string(),
        from_alias: None,
        joins: Vec::new(),
        where_clause,
        order_by: None,
        limit: Some(1),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    Subquery {
        select,
        correlations: Vec::new(),
    }
}

// ============================================================================
// Uncorrelated scalar subquery tests
// ============================================================================

#[test]
fn test_scalar_subquery_max_price() {
    let (_dir, collection) = setup_subquery_test_collection();

    // (SELECT MAX(price) FROM products) should return 50
    let subquery = make_aggregate_subquery(velesql::AggregateType::Max, "price", "products", None);

    let params = HashMap::new();
    let result = collection
        .execute_scalar_subquery(&subquery, &params, None)
        .expect("execute_scalar_subquery failed");

    // MAX(price) = 50 → should return Value::Integer(50) or Value::Float(50.0)
    match result {
        Value::Integer(i) => assert_eq!(i, 50, "MAX(price) should be 50"),
        Value::Float(f) => assert!((f - 50.0).abs() < 0.01, "MAX(price) should be 50.0"),
        other => panic!("Expected numeric value for MAX(price), got {:?}", other),
    }
}

#[test]
fn test_scalar_subquery_avg_price() {
    let (_dir, collection) = setup_subquery_test_collection();

    // (SELECT AVG(price) FROM products) should return 30.0
    // prices: 10, 20, 30, 40, 50 → avg = 30.0
    let subquery = make_aggregate_subquery(velesql::AggregateType::Avg, "price", "products", None);

    let params = HashMap::new();
    let result = collection
        .execute_scalar_subquery(&subquery, &params, None)
        .expect("execute_scalar_subquery failed");

    match result {
        Value::Float(f) => assert!(
            (f - 30.0).abs() < 0.01,
            "AVG(price) should be 30.0, got {}",
            f
        ),
        Value::Integer(i) => assert_eq!(i, 30, "AVG(price) should be 30"),
        other => panic!("Expected numeric value for AVG(price), got {:?}", other),
    }
}

#[test]
fn test_scalar_subquery_no_results_returns_null() {
    let (_dir, collection) = setup_subquery_test_collection();

    // (SELECT MAX(price) FROM products WHERE category = 'nonexistent')
    // No matching rows → should return Value::Null
    let where_clause = Condition::Comparison(velesql::Comparison {
        column: "category".to_string(),
        operator: velesql::CompareOp::Eq,
        value: Value::String("nonexistent".to_string()),
    });

    let subquery = make_aggregate_subquery(
        velesql::AggregateType::Max,
        "price",
        "products",
        Some(where_clause),
    );

    let params = HashMap::new();
    let result = collection
        .execute_scalar_subquery(&subquery, &params, None)
        .expect("execute_scalar_subquery failed");

    assert_eq!(
        result,
        Value::Null,
        "Subquery with no matching rows should return Null"
    );
}

#[test]
fn test_scalar_subquery_multi_row_uses_first() {
    let (_dir, collection) = setup_subquery_test_collection();

    // (SELECT price FROM products LIMIT 1) — returns first row's price
    // We just verify it returns a valid numeric value (not Null)
    let subquery = make_scalar_select_subquery("price", "products", None);

    let params = HashMap::new();
    let result = collection
        .execute_scalar_subquery(&subquery, &params, None)
        .expect("execute_scalar_subquery failed");

    // Should return some numeric value (whichever row comes first)
    assert_ne!(
        result,
        Value::Null,
        "Scalar subquery should return a value, not Null"
    );
}

#[test]
fn test_scalar_subquery_with_where_filter() {
    let (_dir, collection) = setup_subquery_test_collection();

    // (SELECT AVG(price) FROM products WHERE category = 'electronics')
    // electronics prices: 10, 20, 40 → avg = 23.33...
    let where_clause = Condition::Comparison(velesql::Comparison {
        column: "category".to_string(),
        operator: velesql::CompareOp::Eq,
        value: Value::String("electronics".to_string()),
    });

    let subquery = make_aggregate_subquery(
        velesql::AggregateType::Avg,
        "price",
        "products",
        Some(where_clause),
    );

    let params = HashMap::new();
    let result = collection
        .execute_scalar_subquery(&subquery, &params, None)
        .expect("execute_scalar_subquery failed");

    match result {
        Value::Float(f) => {
            // avg of 10, 20, 40 = 23.33
            assert!(
                (f - 23.33).abs() < 0.5,
                "AVG(price) for electronics should be ~23.33, got {}",
                f
            );
        }
        other => panic!("Expected Float for AVG, got {:?}", other),
    }
}

// ============================================================================
// resolve_value helper tests
// ============================================================================

#[test]
fn test_resolve_value_non_subquery_passthrough() {
    let (_dir, collection) = setup_subquery_test_collection();
    let params = HashMap::new();

    // Non-subquery values should pass through unchanged
    let int_val = Value::Integer(42);
    let resolved = collection
        .resolve_subquery_value(&int_val, &params, None)
        .expect("resolve_subquery_value failed");
    assert_eq!(resolved, Value::Integer(42));

    let str_val = Value::String("hello".to_string());
    let resolved = collection
        .resolve_subquery_value(&str_val, &params, None)
        .expect("resolve_subquery_value failed");
    assert_eq!(resolved, Value::String("hello".to_string()));

    let null_val = Value::Null;
    let resolved = collection
        .resolve_subquery_value(&null_val, &params, None)
        .expect("resolve_subquery_value failed");
    assert_eq!(resolved, Value::Null);
}

#[test]
fn test_resolve_value_subquery_executes() {
    let (_dir, collection) = setup_subquery_test_collection();

    // resolve_value with a Subquery should execute it and return concrete value
    let subquery = make_aggregate_subquery(velesql::AggregateType::Max, "price", "products", None);

    let subquery_value = Value::Subquery(Box::new(subquery));
    let params = HashMap::new();
    let result = collection
        .resolve_subquery_value(&subquery_value, &params, None)
        .expect("resolve_subquery_value failed");

    // Should resolve to a concrete value, not Null or Subquery
    assert!(
        !matches!(result, Value::Subquery(_)),
        "Subquery should be resolved"
    );
    assert_ne!(result, Value::Null, "MAX(price) should not be Null");
}

// ============================================================================
// Correlated subquery tests
// ============================================================================

#[test]
fn test_correlated_subquery_with_outer_context() {
    let (_dir, collection) = setup_subquery_test_collection();

    // Simulate: (SELECT AVG(price) FROM products WHERE category = $category)
    // where outer_row has category = 'electronics'
    // Reason: The correlated column injects outer_row["category"] as param $category
    let where_clause = Condition::Comparison(velesql::Comparison {
        column: "category".to_string(),
        operator: velesql::CompareOp::Eq,
        value: Value::Parameter("category".to_string()),
    });

    let subquery = Subquery {
        select: velesql::SelectStatement {
            distinct: velesql::DistinctMode::None,
            columns: velesql::SelectColumns::Aggregations(vec![velesql::AggregateFunction {
                function_type: velesql::AggregateType::Avg,
                argument: velesql::AggregateArg::Column("price".to_string()),
                alias: None,
            }]),
            from: "products".to_string(),
            from_alias: None,
            joins: Vec::new(),
            where_clause: Some(where_clause),
            order_by: None,
            limit: None,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        correlations: vec![velesql::CorrelatedColumn {
            outer_table: "orders".to_string(),
            outer_column: "category".to_string(),
            inner_column: "orders.category".to_string(),
        }],
    };

    // The outer row provides the value for the correlated column
    let outer_row = json!({"category": "electronics", "name": "Order 1"});
    let params = HashMap::new();

    let result = collection
        .execute_scalar_subquery(&subquery, &params, Some(&outer_row))
        .expect("execute_scalar_subquery failed");

    match result {
        Value::Float(f) => {
            // electronics prices: 10, 20, 40 → avg = 23.33
            assert!(
                (f - 23.33).abs() < 0.5,
                "Correlated AVG should be ~23.33, got {}",
                f
            );
        }
        other => panic!("Expected Float for correlated AVG, got {:?}", other),
    }
}

// ============================================================================
// VP-002: SELECT WHERE with subquery integration tests
// ============================================================================

#[test]
fn test_select_where_with_subquery() {
    let (_dir, collection) = setup_subquery_test_collection();

    // SELECT * FROM products WHERE price < (SELECT AVG(price) FROM products)
    // Prices: 10, 20, 30, 40, 50 → AVG = 30.0
    // Products with price < 30: Widget A (10), Widget B (20)
    let subquery = make_aggregate_subquery(velesql::AggregateType::Avg, "price", "products", None);

    let select = velesql::SelectStatement {
        distinct: velesql::DistinctMode::None,
        columns: velesql::SelectColumns::All,
        from: "products".to_string(),
        from_alias: None,
        joins: Vec::new(),
        where_clause: Some(Condition::Comparison(velesql::Comparison {
            column: "price".to_string(),
            operator: velesql::CompareOp::Lt,
            value: Value::Subquery(Box::new(subquery)),
        })),
        order_by: None,
        limit: Some(100),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let query = velesql::Query::new_select(select);
    let params = HashMap::new();
    let results = collection
        .execute_query(&query, &params)
        .expect("execute_query with subquery WHERE failed");

    // AVG(price) = 30.0 → price < 30 matches: 10 and 20
    assert_eq!(
        results.len(),
        2,
        "SELECT WHERE price < (SELECT AVG(price)) should return 2 rows, got {}",
        results.len()
    );

    let prices: Vec<i64> = results
        .iter()
        .filter_map(|r| r.point.payload.as_ref()?.get("price")?.as_i64())
        .collect();
    assert!(prices.contains(&10), "Price 10 should be in results");
    assert!(prices.contains(&20), "Price 20 should be in results");
}

#[test]
fn test_select_where_subquery_null_result() {
    let (_dir, collection) = setup_subquery_test_collection();

    // SELECT * FROM products WHERE price < (SELECT MAX(price) FROM products WHERE category = 'nonexistent')
    // Subquery returns Null → comparison "price < Null" is false for all → 0 results
    let where_clause = Condition::Comparison(velesql::Comparison {
        column: "category".to_string(),
        operator: velesql::CompareOp::Eq,
        value: Value::String("nonexistent".to_string()),
    });

    let subquery = make_aggregate_subquery(
        velesql::AggregateType::Max,
        "price",
        "products",
        Some(where_clause),
    );

    let select = velesql::SelectStatement {
        distinct: velesql::DistinctMode::None,
        columns: velesql::SelectColumns::All,
        from: "products".to_string(),
        from_alias: None,
        joins: Vec::new(),
        where_clause: Some(Condition::Comparison(velesql::Comparison {
            column: "price".to_string(),
            operator: velesql::CompareOp::Lt,
            value: Value::Subquery(Box::new(subquery)),
        })),
        order_by: None,
        limit: Some(100),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let query = velesql::Query::new_select(select);
    let params = HashMap::new();
    let results = collection
        .execute_query(&query, &params)
        .expect("execute_query with null subquery failed");

    // Subquery returns Null → "price < Null" is false → 0 results (not all results)
    assert_eq!(
        results.len(),
        0,
        "SELECT WHERE price < (null subquery) should return 0 rows, got {}",
        results.len()
    );
}
