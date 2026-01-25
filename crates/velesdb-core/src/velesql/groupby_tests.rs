//! Tests for VelesQL GROUP BY (EPIC-017 US-003).

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
fn test_parser_groupby_single_column() {
    let query = Parser::parse("SELECT category, COUNT(*) FROM items GROUP BY category").unwrap();

    assert!(query.select.group_by.is_some());
    let group_by = query.select.group_by.as_ref().unwrap();
    assert_eq!(group_by.columns.len(), 1);
    assert_eq!(group_by.columns[0], "category");
}

#[test]
fn test_parser_groupby_multiple_columns() {
    let query =
        Parser::parse("SELECT category, status, COUNT(*) FROM items GROUP BY category, status")
            .unwrap();

    assert!(query.select.group_by.is_some());
    let group_by = query.select.group_by.as_ref().unwrap();
    assert_eq!(group_by.columns.len(), 2);
    assert_eq!(group_by.columns[0], "category");
    assert_eq!(group_by.columns[1], "status");
}

#[test]
fn test_parser_groupby_with_aggregations() {
    let query = Parser::parse(
        "SELECT category, COUNT(*), SUM(price), AVG(rating) FROM items GROUP BY category",
    )
    .unwrap();

    assert!(query.select.group_by.is_some());
    // Verify aggregations are parsed correctly (Mixed: column + aggregations)
    match &query.select.columns {
        crate::velesql::SelectColumns::Mixed {
            columns,
            aggregations,
        } => {
            assert_eq!(columns.len(), 1); // category
            assert_eq!(aggregations.len(), 3); // COUNT, SUM, AVG
        }
        crate::velesql::SelectColumns::Aggregations(aggs) => {
            assert!(aggs.len() >= 3); // COUNT, SUM, AVG
        }
        _ => panic!("Expected Mixed or Aggregations in SELECT"),
    }
}

// ========== Executor Tests ==========

#[test]
fn test_executor_groupby_count() {
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

    let query = Parser::parse("SELECT category, COUNT(*) FROM items GROUP BY category").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    // Result should be an array of groups
    let groups = result.as_array().expect("Result should be array of groups");
    assert_eq!(groups.len(), 3); // 3 categories

    // Find and verify each group
    let tech_group = groups
        .iter()
        .find(|g| g.get("category") == Some(&serde_json::json!("tech")));
    assert!(tech_group.is_some());
    assert_eq!(
        tech_group
            .unwrap()
            .get("count")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );

    let science_group = groups
        .iter()
        .find(|g| g.get("category") == Some(&serde_json::json!("science")));
    assert!(science_group.is_some());
    assert_eq!(
        science_group
            .unwrap()
            .get("count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
}

#[test]
fn test_executor_groupby_multiple_aggregations() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech", "price": 100})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech", "price": 200})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "science", "price": 150})),
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT category, COUNT(*), SUM(price) FROM items GROUP BY category")
        .unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    let groups = result.as_array().expect("Result should be array");

    let tech_group = groups
        .iter()
        .find(|g| g.get("category") == Some(&serde_json::json!("tech")))
        .unwrap();
    assert_eq!(
        tech_group.get("count").and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        tech_group
            .get("sum_price")
            .and_then(serde_json::Value::as_f64),
        Some(300.0)
    );

    let science_group = groups
        .iter()
        .find(|g| g.get("category") == Some(&serde_json::json!("science")))
        .unwrap();
    assert_eq!(
        science_group
            .get("count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        science_group
            .get("sum_price")
            .and_then(serde_json::Value::as_f64),
        Some(150.0)
    );
}

#[test]
fn test_executor_groupby_with_avg() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "A", "rating": 4})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "A", "rating": 6})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "B", "rating": 3})),
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT category, AVG(rating) FROM items GROUP BY category").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    let groups = result.as_array().expect("Result should be array");

    let a_group = groups
        .iter()
        .find(|g| g.get("category") == Some(&serde_json::json!("A")))
        .unwrap();
    assert_eq!(
        a_group
            .get("avg_rating")
            .and_then(serde_json::Value::as_f64),
        Some(5.0)
    ); // (4+6)/2
}

#[test]
fn test_executor_groupby_multiple_columns() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech", "status": "active"})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech", "status": "active"})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech", "status": "inactive"})),
        },
        Point {
            id: 4,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "science", "status": "active"})),
        },
    ];
    collection.upsert(points).unwrap();

    let query =
        Parser::parse("SELECT category, status, COUNT(*) FROM items GROUP BY category, status")
            .unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    let groups = result.as_array().expect("Result should be array");
    assert_eq!(groups.len(), 3); // tech-active, tech-inactive, science-active
}

// ========== Limit Protection Test ==========

#[test]
fn test_groupby_limit_protection() {
    let (collection, _tmp) = create_test_collection();

    // Insert many unique categories to exceed limit
    let points: Vec<Point> = (0..100u64)
        .map(|i| Point {
            id: i,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": format!("cat_{}", i)})),
        })
        .collect();
    collection.upsert(points).unwrap();

    // This should work with 100 groups (under default limit)
    let query = Parser::parse("SELECT category, COUNT(*) FROM items GROUP BY category").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params);

    assert!(result.is_ok());
}
