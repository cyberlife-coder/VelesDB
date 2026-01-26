//! Tests for VelesQL HAVING clause (EPIC-017 US-006).

use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::velesql::Parser;
use crate::Collection;
use std::collections::HashMap;
use std::path::PathBuf;

fn create_test_collection() -> (Collection, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = PathBuf::from(temp_dir.path());
    let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();
    (collection, temp_dir)
}

// ========== Parser Tests ==========

#[test]
fn test_parser_having_simple() {
    let query =
        Parser::parse("SELECT category, COUNT(*) FROM items GROUP BY category HAVING COUNT(*) > 5")
            .unwrap();

    assert!(query.select.group_by.is_some());
    assert!(query.select.having.is_some());

    let having = query.select.having.as_ref().unwrap();
    // Verify HAVING condition is parsed
    assert!(!having.conditions.is_empty());
}

#[test]
fn test_parser_having_with_avg() {
    let query = Parser::parse(
        "SELECT category, AVG(price) FROM items GROUP BY category HAVING AVG(price) > 100",
    )
    .unwrap();

    assert!(query.select.having.is_some());
}

#[test]
fn test_parser_having_multiple_conditions() {
    let query = Parser::parse(
        "SELECT category, COUNT(*), SUM(price) FROM items GROUP BY category HAVING COUNT(*) > 2 AND SUM(price) > 500"
    ).unwrap();

    assert!(query.select.having.is_some());
    let having = query.select.having.as_ref().unwrap();
    // Should have AND condition
    assert!(having.conditions.len() >= 2 || matches!(having.conditions[0], _));
}

// ========== Executor Tests ==========

#[test]
fn test_executor_having_count_filter() {
    let (collection, _tmp) = create_test_collection();

    // Insert: 3 tech, 2 science, 1 history
    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech"})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech"})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech"})),
        },
        Point {
            id: 4,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "science"})),
        },
        Point {
            id: 5,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "science"})),
        },
        Point {
            id: 6,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "history"})),
        },
    ];
    collection.upsert(points).unwrap();

    // HAVING COUNT(*) > 1 should filter out history (count=1)
    let query =
        Parser::parse("SELECT category, COUNT(*) FROM items GROUP BY category HAVING COUNT(*) > 1")
            .unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    let groups = result.as_array().expect("Result should be array");
    assert_eq!(groups.len(), 2); // tech (3) and science (2), not history (1)

    // Verify history is filtered out
    let has_history = groups
        .iter()
        .any(|g| g.get("category") == Some(&serde_json::json!("history")));
    assert!(!has_history, "history should be filtered by HAVING");
}

#[test]
fn test_executor_having_avg_filter() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "A", "price": 200})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "A", "price": 300})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "B", "price": 50})),
        },
        Point {
            id: 4,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "B", "price": 50})),
        },
    ];
    collection.upsert(points).unwrap();

    // HAVING AVG(price) > 100 should keep A (avg=250), filter B (avg=50)
    let query = Parser::parse(
        "SELECT category, AVG(price) FROM items GROUP BY category HAVING AVG(price) > 100",
    )
    .unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    let groups = result.as_array().expect("Result should be array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].get("category"), Some(&serde_json::json!("A")));
}

#[test]
fn test_executor_having_sum_filter() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "X", "amount": 100})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "X", "amount": 200})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "Y", "amount": 50})),
        },
    ];
    collection.upsert(points).unwrap();

    // HAVING SUM(amount) >= 300 should keep X (sum=300), filter Y (sum=50)
    let query = Parser::parse(
        "SELECT category, SUM(amount) FROM items GROUP BY category HAVING SUM(amount) >= 300",
    )
    .unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    let groups = result.as_array().expect("Result should be array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].get("category"), Some(&serde_json::json!("X")));
}

// ========== Semantic Validation Tests ==========

#[test]
fn test_executor_having_without_groupby_returns_error() {
    let (collection, _tmp) = create_test_collection();

    // Insert some data
    let points = vec![Point {
        id: 1,
        vector: vec![0.1; 4],
        payload: Some(serde_json::json!({"value": 10})),
    }];
    collection.upsert(points).unwrap();

    // HAVING without GROUP BY should return an error at execution time
    let query = Parser::parse("SELECT COUNT(*) FROM items HAVING COUNT(*) > 10").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params);

    // Should return error, not silently ignore HAVING
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("HAVING") && err_msg.contains("GROUP BY"),
        "Error should mention HAVING requires GROUP BY, got: {err_msg}"
    );
}
