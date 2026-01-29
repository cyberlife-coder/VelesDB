//! Tests for DISTINCT keyword support (EPIC-052 US-001).
#![cfg(all(test, feature = "persistence"))]

use crate::collection::Collection;
use crate::velesql::{DistinctMode, Parser};
use crate::Point;
use std::collections::HashMap;
use tempfile::TempDir;

fn create_test_collection() -> (Collection, TempDir) {
    let tmp = TempDir::new().unwrap();
    let collection =
        Collection::create(tmp.path().to_path_buf(), 4, crate::DistanceMetric::Cosine).unwrap();
    (collection, tmp)
}

// ========== Parser Tests ==========

#[test]
fn test_parse_distinct_basic() {
    let query = Parser::parse("SELECT DISTINCT category FROM products").unwrap();
    assert_eq!(query.select.distinct, DistinctMode::All);
}

#[test]
fn test_parse_distinct_multiple_columns() {
    let query = Parser::parse("SELECT DISTINCT category, brand FROM products").unwrap();
    assert_eq!(query.select.distinct, DistinctMode::All);
}

#[test]
fn test_parse_no_distinct() {
    let query = Parser::parse("SELECT category FROM products").unwrap();
    assert_eq!(query.select.distinct, DistinctMode::None);
}

#[test]
fn test_parse_distinct_star() {
    let query = Parser::parse("SELECT DISTINCT * FROM products").unwrap();
    assert_eq!(query.select.distinct, DistinctMode::All);
}

#[test]
fn test_parse_distinct_with_where() {
    let query = Parser::parse("SELECT DISTINCT brand FROM products WHERE category = 'electronics'")
        .unwrap();
    assert_eq!(query.select.distinct, DistinctMode::All);
}

#[test]
fn test_parse_distinct_with_order_by() {
    let query =
        Parser::parse("SELECT DISTINCT category FROM products ORDER BY category ASC").unwrap();
    assert_eq!(query.select.distinct, DistinctMode::All);
}

#[test]
fn test_parse_distinct_with_limit() {
    let query = Parser::parse("SELECT DISTINCT category FROM products LIMIT 5").unwrap();
    assert_eq!(query.select.distinct, DistinctMode::All);
}

// ========== Executor Tests ==========

#[test]
fn test_executor_distinct_single_column() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "electronics", "brand": "Apple"})),
        },
        Point {
            id: 2,
            vector: vec![0.2; 4],
            payload: Some(serde_json::json!({"category": "electronics", "brand": "Samsung"})),
        },
        Point {
            id: 3,
            vector: vec![0.3; 4],
            payload: Some(serde_json::json!({"category": "clothing", "brand": "Nike"})),
        },
        Point {
            id: 4,
            vector: vec![0.4; 4],
            payload: Some(serde_json::json!({"category": "electronics", "brand": "Apple"})),
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT DISTINCT category FROM products").unwrap();
    let params = HashMap::new();
    let results = collection.execute_query(&query, &params).unwrap();

    // Should have 2 distinct categories: electronics and clothing
    let categories: std::collections::HashSet<_> = results
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("category"))
                .and_then(|v| v.as_str())
        })
        .collect();

    assert_eq!(categories.len(), 2);
    assert!(categories.contains("electronics"));
    assert!(categories.contains("clothing"));
}

#[test]
fn test_executor_distinct_multiple_columns() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "electronics", "brand": "Apple"})),
        },
        Point {
            id: 2,
            vector: vec![0.2; 4],
            payload: Some(serde_json::json!({"category": "electronics", "brand": "Apple"})),
        },
        Point {
            id: 3,
            vector: vec![0.3; 4],
            payload: Some(serde_json::json!({"category": "electronics", "brand": "Samsung"})),
        },
        Point {
            id: 4,
            vector: vec![0.4; 4],
            payload: Some(serde_json::json!({"category": "clothing", "brand": "Nike"})),
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT DISTINCT category, brand FROM products").unwrap();
    let params = HashMap::new();
    let results = collection.execute_query(&query, &params).unwrap();

    // Should have 3 distinct (category, brand) pairs
    assert_eq!(results.len(), 3);
}

#[test]
fn test_executor_distinct_preserves_order() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "A"})),
        },
        Point {
            id: 2,
            vector: vec![0.2; 4],
            payload: Some(serde_json::json!({"category": "B"})),
        },
        Point {
            id: 3,
            vector: vec![0.3; 4],
            payload: Some(serde_json::json!({"category": "A"})),
        },
        Point {
            id: 4,
            vector: vec![0.4; 4],
            payload: Some(serde_json::json!({"category": "C"})),
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT DISTINCT category FROM products").unwrap();
    let params = HashMap::new();
    let results = collection.execute_query(&query, &params).unwrap();

    // First occurrence order should be preserved: A, B, C
    let categories: Vec<_> = results
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("category"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();

    assert_eq!(categories.len(), 3);
    // A should appear before B (first seen at id=1)
    let a_pos = categories.iter().position(|c| c == "A").unwrap();
    let b_pos = categories.iter().position(|c| c == "B").unwrap();
    assert!(a_pos < b_pos);
}

#[test]
fn test_executor_distinct_with_null() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "A"})),
        },
        Point {
            id: 2,
            vector: vec![0.2; 4],
            payload: Some(serde_json::json!({})), // No category
        },
        Point {
            id: 3,
            vector: vec![0.3; 4],
            payload: Some(serde_json::json!({"category": "A"})),
        },
        Point {
            id: 4,
            vector: vec![0.4; 4],
            payload: None, // No payload
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT DISTINCT category FROM products").unwrap();
    let params = HashMap::new();
    let results = collection.execute_query(&query, &params).unwrap();

    // Should have 2 distinct values: "A" and null (consolidated)
    assert_eq!(results.len(), 2);
}

#[test]
fn test_executor_distinct_empty_result() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![Point {
        id: 1,
        vector: vec![0.1; 4],
        payload: Some(serde_json::json!({"category": "A", "price": 100})),
    }];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT DISTINCT category FROM products WHERE price > 1000").unwrap();
    let params = HashMap::new();
    let results = collection.execute_query(&query, &params).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_executor_no_distinct_returns_all() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "A"})),
        },
        Point {
            id: 2,
            vector: vec![0.2; 4],
            payload: Some(serde_json::json!({"category": "A"})),
        },
        Point {
            id: 3,
            vector: vec![0.3; 4],
            payload: Some(serde_json::json!({"category": "A"})),
        },
    ];
    collection.upsert(points).unwrap();

    // Without DISTINCT, should return all rows
    let query = Parser::parse("SELECT category FROM products").unwrap();
    let params = HashMap::new();
    let results = collection.execute_query(&query, &params).unwrap();

    assert_eq!(results.len(), 3);
}

// ========== Serialization Tests ==========

#[test]
fn test_distinct_mode_serialization() {
    let query = Parser::parse("SELECT DISTINCT category FROM products").unwrap();
    let json = serde_json::to_string(&query).unwrap();
    let parsed: crate::velesql::Query = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.select.distinct, DistinctMode::All);
}

#[test]
fn test_distinct_mode_none_serialization() {
    let query = Parser::parse("SELECT category FROM products").unwrap();
    let json = serde_json::to_string(&query).unwrap();
    let parsed: crate::velesql::Query = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.select.distinct, DistinctMode::None);
}
