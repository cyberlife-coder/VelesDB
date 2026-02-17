//! Tests for `VelesQL` aggregation execution (EPIC-017 US-002).
#![cfg(all(test, feature = "persistence"))]

use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::velesql::{Aggregator, Parser};
use crate::Collection;
use std::collections::HashMap;
use std::path::PathBuf;

fn create_test_collection() -> (Collection, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = PathBuf::from(temp_dir.path());
    let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();
    (collection, temp_dir)
}

#[test]
fn test_executor_count_star() {
    let (collection, _tmp) = create_test_collection();

    // Insert test data
    let points: Vec<Point> = (0..100u64)
        .map(|i| Point {
            id: i,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"id": i})),
        })
        .collect();
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT COUNT(*) FROM documents").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    assert_eq!(
        result.get("count").and_then(serde_json::Value::as_u64),
        Some(100)
    );
}

#[test]
fn test_executor_count_with_filter() {
    let (collection, _tmp) = create_test_collection();

    // Insert 50 tech + 50 science
    let mut points: Vec<Point> = (0..50u64)
        .map(|i| Point {
            id: i,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"category": "tech"})),
        })
        .collect();
    points.extend((50..100u64).map(|i| Point {
        id: i,
        vector: vec![0.1; 4],
        payload: Some(serde_json::json!({"category": "science"})),
    }));
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT COUNT(*) FROM items WHERE category = 'tech'").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    assert_eq!(
        result.get("count").and_then(serde_json::Value::as_u64),
        Some(50)
    );
}

#[test]
fn test_executor_sum() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"price": 10})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"price": 20})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"price": 30})),
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT SUM(price) FROM orders").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    assert_eq!(
        result.get("sum_price").and_then(serde_json::Value::as_f64),
        Some(60.0)
    );
}

#[test]
fn test_executor_avg() {
    let (collection, _tmp) = create_test_collection();

    let points = vec![
        Point {
            id: 1,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"rating": 3})),
        },
        Point {
            id: 2,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"rating": 4})),
        },
        Point {
            id: 3,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"rating": 5})),
        },
    ];
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT AVG(rating) FROM reviews").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    assert_eq!(
        result.get("avg_rating").and_then(serde_json::Value::as_f64),
        Some(4.0)
    );
}

#[test]
fn test_executor_min_max() {
    let (collection, _tmp) = create_test_collection();

    let points: Vec<Point> = [1, 5, 3, 9, 2]
        .iter()
        .enumerate()
        .map(|(i, val)| Point {
            id: i as u64,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"value": val})),
        })
        .collect();
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT MIN(value), MAX(value) FROM items").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    assert_eq!(
        result.get("min_value").and_then(serde_json::Value::as_f64),
        Some(1.0)
    );
    assert_eq!(
        result.get("max_value").and_then(serde_json::Value::as_f64),
        Some(9.0)
    );
}

#[test]
fn test_executor_multiple_aggregations() {
    let (collection, _tmp) = create_test_collection();

    let points: Vec<Point> = (1..=10u64)
        .map(|i| Point {
            id: i,
            vector: vec![0.1; 4],
            payload: Some(serde_json::json!({"score": i})),
        })
        .collect();
    collection.upsert(points).unwrap();

    let query = Parser::parse("SELECT COUNT(*), SUM(score), AVG(score) FROM results").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    assert_eq!(
        result.get("count").and_then(serde_json::Value::as_u64),
        Some(10)
    );
    assert_eq!(
        result.get("sum_score").and_then(serde_json::Value::as_f64),
        Some(55.0)
    );
    assert_eq!(
        result.get("avg_score").and_then(serde_json::Value::as_f64),
        Some(5.5)
    );
}

#[test]
fn test_executor_empty_collection() {
    let (collection, _tmp) = create_test_collection();

    let query = Parser::parse("SELECT COUNT(*), SUM(price) FROM empty").unwrap();
    let params = HashMap::new();
    let result = collection.execute_aggregate(&query, &params).unwrap();

    assert_eq!(
        result.get("count").and_then(serde_json::Value::as_u64),
        Some(0)
    );
    // SUM of empty set should be null or not present
    // SUM of empty set returns null, which is valid JSON null
    let sum_price = result.get("sum_price");
    assert!(sum_price.is_none() || sum_price.is_some_and(serde_json::Value::is_null));
}

#[test]
fn test_aggregator_streaming() {
    // Test the Aggregator struct directly for streaming behavior
    let mut aggregator = Aggregator::new();

    aggregator.process_count();
    aggregator.process_value("price", &serde_json::json!(10));
    aggregator.process_count();
    aggregator.process_value("price", &serde_json::json!(20));
    aggregator.process_count();
    aggregator.process_value("price", &serde_json::json!(30));

    let result = aggregator.finalize();

    assert_eq!(result.count, 3);
    assert_eq!(result.sums.get("price"), Some(&60.0));
}
