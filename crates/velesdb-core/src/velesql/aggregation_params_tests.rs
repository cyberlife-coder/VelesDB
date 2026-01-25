//! Bug 5 Fix Tests: Aggregation params resolution
//!
//! Tests that params are properly resolved in execute_aggregate queries.

use crate::collection::Collection;
use crate::velesql::Parser;
use std::collections::HashMap;
use tempfile::TempDir;

/// BUG 5: Aggregation params become NULL
/// The params parameter in execute_aggregate was prefixed with _ and ignored.
/// Placeholders like $cat in WHERE clauses were never resolved.
#[test]
fn test_bug_5_aggregation_params_should_be_resolved() {
    // Create a test collection
    let temp_dir = TempDir::new().unwrap();
    let collection = Collection::create(
        temp_dir.path().to_path_buf(),
        4,
        crate::distance::DistanceMetric::Cosine,
    )
    .unwrap();

    // Insert test data with different categories
    collection
        .upsert(vec![crate::point::Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"category": "electronics", "price": 100.0})),
        }])
        .unwrap();

    collection
        .upsert(vec![crate::point::Point {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"category": "electronics", "price": 200.0})),
        }])
        .unwrap();

    collection
        .upsert(vec![crate::point::Point {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: Some(serde_json::json!({"category": "books", "price": 50.0})),
        }])
        .unwrap();

    // Parse query with placeholder
    let sql = "SELECT COUNT(*), SUM(price) FROM products WHERE category = $cat";
    let query = Parser::parse(sql).expect("Query should parse");

    // Create params with the category value
    let mut params: HashMap<String, serde_json::Value> = HashMap::new();
    params.insert("cat".to_string(), serde_json::json!("electronics"));

    // Execute aggregation with params
    let result = collection.execute_aggregate(&query, &params);

    assert!(
        result.is_ok(),
        "Aggregation should succeed: {:?}",
        result.err()
    );

    let json = result.unwrap();

    // Should only count electronics (2 items), not all (3 items)
    // If params are ignored, it would either:
    // - Return 0 (no match for literal "$cat")
    // - Or return 3 (no filter applied)
    let count = json
        .get("count")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    assert_eq!(
        count, 2,
        "Should count only 'electronics' category (2 items). Got {}. Params may not be resolved.",
        count
    );

    // Sum should be 100 + 200 = 300, not 350 (all items)
    let sum = json
        .get("sum_price")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    assert!(
        (sum - 300.0).abs() < 0.01,
        "Sum should be 300.0 for electronics. Got {}",
        sum
    );
}

/// Test that params work with grouped aggregation too
#[test]
fn test_bug_5_grouped_aggregation_with_params() {
    let temp_dir = TempDir::new().unwrap();
    let collection = Collection::create(
        temp_dir.path().to_path_buf(),
        4,
        crate::distance::DistanceMetric::Cosine,
    )
    .unwrap();

    // Insert test data
    for i in 0u64..5 {
        collection
            .upsert(vec![crate::point::Point {
                id: i,
                vector: vec![i as f32, 0.0, 0.0, 0.0],
                payload: Some(serde_json::json!({
                    "category": if i < 3 { "A" } else { "B" },
                    "price": (i + 1) * 10
                })),
            }])
            .unwrap();
    }

    // Query with param for minimum price filter
    let sql = "SELECT category, COUNT(*) FROM products WHERE price > $min GROUP BY category";
    let query = Parser::parse(sql).expect("Query should parse");

    let mut params: HashMap<String, serde_json::Value> = HashMap::new();
    params.insert("min".to_string(), serde_json::json!(20)); // Only items with price > 20

    let result = collection.execute_aggregate(&query, &params);

    assert!(
        result.is_ok(),
        "Grouped aggregation with params should work: {:?}",
        result.err()
    );
}
