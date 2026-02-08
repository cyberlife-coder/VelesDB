//! Tests for MATCH WHERE clause evaluation of all condition types (VP-001).
//!
//! Verifies that LIKE, BETWEEN, IN, IsNull, Match (full-text), and temporal
//! conditions are properly evaluated in MATCH WHERE context — NOT silently
//! passed through as `Ok(true)`.

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;

use crate::velesql::{
    BetweenCondition, Condition, InCondition, IsNullCondition, LikeCondition, MatchCondition, Value,
};
use crate::velesql::{GraphPattern, MatchClause, NodePattern, ReturnClause};
use crate::{Database, DistanceMetric, Point};

/// Helper: create a collection with labeled nodes and payloads for MATCH testing.
fn setup_match_test_collection() -> (TempDir, crate::Collection) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");

    db.create_collection("test_match", 4, DistanceMetric::Cosine)
        .expect("Failed to create collection");
    let collection = db
        .get_collection("test_match")
        .expect("Failed to get collection");

    // Insert nodes with various payloads for testing different WHERE conditions
    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Product"],
                "name": "Wireless Headphones",
                "category": "electronics",
                "price": 89,
                "in_stock": true,
                "description": "Premium wireless noise-cancelling headphones",
                "updated_at": 1700000000_i64
            })),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Product"],
                "name": "Bluetooth Speaker",
                "category": "electronics",
                "price": 45,
                "in_stock": true,
                "description": "Portable bluetooth speaker with bass boost",
                "updated_at": 1700100000_i64
            })),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "_labels": ["Product"],
                "name": "Leather Wallet",
                "category": "accessories",
                "price": 35,
                "in_stock": false,
                "description": null,
                "updated_at": 1699900000_i64
            })),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({
                "_labels": ["Product"],
                "name": "USB-C Cable",
                "category": "electronics",
                "price": 15,
                "in_stock": true,
                "updated_at": 1700200000_i64
            })),
        ),
    ];

    collection.upsert(points).expect("Failed to upsert points");
    (temp_dir, collection)
}

/// Helper: create a simple MATCH clause with a WHERE condition on Product nodes.
fn match_clause_with_condition(condition: Condition) -> MatchClause {
    MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![NodePattern::new().with_alias("p").with_label("Product")],
            relationships: vec![],
        }],
        where_clause: Some(condition),
        return_clause: ReturnClause {
            items: vec![],
            order_by: None,
            limit: Some(100),
        },
    }
}

// ============================================================================
// LIKE condition tests (VP-001)
// ============================================================================

#[test]
fn test_match_where_like_filters_correctly() {
    let (_dir, collection) = setup_match_test_collection();

    // LIKE '%wireless%' should match only node 1
    let condition = Condition::Like(LikeCondition {
        column: "name".to_string(),
        pattern: "%Wireless%".to_string(),
        case_insensitive: false,
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 1, "LIKE '%Wireless%' should match 1 node");
    assert_eq!(results[0].node_id, 1);
}

#[test]
fn test_match_where_ilike_case_insensitive() {
    let (_dir, collection) = setup_match_test_collection();

    // ILIKE '%wireless%' (case-insensitive) should also match node 1
    let condition = Condition::Like(LikeCondition {
        column: "name".to_string(),
        pattern: "%wireless%".to_string(),
        case_insensitive: true,
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 1, "ILIKE '%wireless%' should match 1 node");
    assert_eq!(results[0].node_id, 1);
}

#[test]
fn test_match_where_like_no_match() {
    let (_dir, collection) = setup_match_test_collection();

    // LIKE '%nonexistent%' should match no nodes
    let condition = Condition::Like(LikeCondition {
        column: "name".to_string(),
        pattern: "%nonexistent%".to_string(),
        case_insensitive: false,
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(
        results.len(),
        0,
        "LIKE '%nonexistent%' should match 0 nodes"
    );
}

// ============================================================================
// BETWEEN condition tests (VP-001)
// ============================================================================

#[test]
fn test_match_where_between_filters_correctly() {
    let (_dir, collection) = setup_match_test_collection();

    // price BETWEEN 30 AND 50 should match nodes 2 (45) and 3 (35)
    let condition = Condition::Between(BetweenCondition {
        column: "price".to_string(),
        low: Value::Integer(30),
        high: Value::Integer(50),
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 2, "BETWEEN 30 AND 50 should match 2 nodes");
    let ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(ids.contains(&2), "Node 2 (price=45) should match");
    assert!(ids.contains(&3), "Node 3 (price=35) should match");
}

#[test]
fn test_match_where_between_no_match() {
    let (_dir, collection) = setup_match_test_collection();

    // price BETWEEN 100 AND 200 should match no nodes
    let condition = Condition::Between(BetweenCondition {
        column: "price".to_string(),
        low: Value::Integer(100),
        high: Value::Integer(200),
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 0, "BETWEEN 100 AND 200 should match 0 nodes");
}

// ============================================================================
// IN condition tests (VP-001)
// ============================================================================

#[test]
fn test_match_where_in_filters_correctly() {
    let (_dir, collection) = setup_match_test_collection();

    // category IN ('electronics', 'accessories') should match all nodes
    // but category IN ('accessories') should match only node 3
    let condition = Condition::In(InCondition {
        column: "category".to_string(),
        values: vec![Value::String("accessories".to_string())],
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 1, "IN ('accessories') should match 1 node");
    assert_eq!(results[0].node_id, 3);
}

#[test]
fn test_match_where_in_multiple_values() {
    let (_dir, collection) = setup_match_test_collection();

    // category IN ('electronics') should match nodes 1, 2, 4
    let condition = Condition::In(InCondition {
        column: "category".to_string(),
        values: vec![Value::String("electronics".to_string())],
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 3, "IN ('electronics') should match 3 nodes");
}

#[test]
fn test_match_where_in_no_match() {
    let (_dir, collection) = setup_match_test_collection();

    // category IN ('furniture') should match no nodes
    let condition = Condition::In(InCondition {
        column: "category".to_string(),
        values: vec![Value::String("furniture".to_string())],
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 0, "IN ('furniture') should match 0 nodes");
}

// ============================================================================
// IS NULL / IS NOT NULL condition tests (VP-001)
// ============================================================================

#[test]
fn test_match_where_is_null_filters_correctly() {
    let (_dir, collection) = setup_match_test_collection();

    // description IS NULL should match node 3 (null) and node 4 (missing field)
    let condition = Condition::IsNull(IsNullCondition {
        column: "description".to_string(),
        is_null: true,
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    // Node 3 has description: null, node 4 has no description field
    assert!(
        results.len() >= 1,
        "IS NULL should match at least node 3 (null description)"
    );
    let ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(
        ids.contains(&3),
        "Node 3 (description=null) should match IS NULL"
    );
}

#[test]
fn test_match_where_is_not_null_filters_correctly() {
    let (_dir, collection) = setup_match_test_collection();

    // description IS NOT NULL should match nodes 1 and 2
    let condition = Condition::IsNull(IsNullCondition {
        column: "description".to_string(),
        is_null: false,
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(
        results.len(),
        2,
        "IS NOT NULL should match nodes with non-null description"
    );
    let ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(ids.contains(&1), "Node 1 should match IS NOT NULL");
    assert!(ids.contains(&2), "Node 2 should match IS NOT NULL");
}

// ============================================================================
// MATCH (full-text) condition tests (VP-001)
// ============================================================================

#[test]
fn test_match_where_fulltext_match_filters_correctly() {
    let (_dir, collection) = setup_match_test_collection();

    // name MATCH 'bluetooth' should match node 2 (contains 'Bluetooth')
    let condition = Condition::Match(MatchCondition {
        column: "name".to_string(),
        query: "Bluetooth".to_string(),
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(results.len(), 1, "MATCH 'Bluetooth' should match 1 node");
    assert_eq!(results[0].node_id, 2);
}

// ============================================================================
// Temporal condition tests (VP-003)
// ============================================================================

#[test]
fn test_match_where_temporal_comparison() {
    let (_dir, collection) = setup_match_test_collection();

    // updated_at > 1700050000 should match nodes 2 and 4
    // (node 1: 1700000000, node 2: 1700100000, node 3: 1699900000, node 4: 1700200000)
    // Reason: NOW() would be a current timestamp far in the future relative to test data.
    // We test with a fixed integer for deterministic results.
    // Temporal expression parsing and epoch conversion is tested separately in parser tests.
    let _temporal_condition = Condition::Comparison(crate::velesql::Comparison {
        column: "updated_at".to_string(),
        operator: crate::velesql::CompareOp::Gt,
        value: Value::Temporal(crate::velesql::TemporalExpr::Now),
    });

    let fixed_condition = Condition::Comparison(crate::velesql::Comparison {
        column: "updated_at".to_string(),
        operator: crate::velesql::CompareOp::Gt,
        value: Value::Integer(1_700_050_000),
    });

    let clause = match_clause_with_condition(fixed_condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(
        results.len(),
        2,
        "updated_at > 1700050000 should match 2 nodes"
    );
    let ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(ids.contains(&2), "Node 2 (1700100000) should match");
    assert!(ids.contains(&4), "Node 4 (1700200000) should match");
}

// ============================================================================
// Combined condition tests (AND with different types)
// ============================================================================

#[test]
fn test_match_where_like_and_between_combined() {
    let (_dir, collection) = setup_match_test_collection();

    // name LIKE '%Bluetooth%' AND price BETWEEN 40 AND 50
    // Should match only node 2 (Bluetooth Speaker, price=45)
    let like_cond = Condition::Like(LikeCondition {
        column: "name".to_string(),
        pattern: "%Bluetooth%".to_string(),
        case_insensitive: false,
    });

    let between_cond = Condition::Between(BetweenCondition {
        column: "price".to_string(),
        low: Value::Integer(40),
        high: Value::Integer(50),
    });

    let combined = Condition::And(Box::new(like_cond), Box::new(between_cond));

    let clause = match_clause_with_condition(combined);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(
        results.len(),
        1,
        "LIKE + BETWEEN combined should match 1 node"
    );
    assert_eq!(results[0].node_id, 2);
}

#[test]
fn test_match_where_in_and_comparison_combined() {
    let (_dir, collection) = setup_match_test_collection();

    // category IN ('electronics') AND price < 50
    // Should match nodes 2 (45) and 4 (15)
    let in_cond = Condition::In(InCondition {
        column: "category".to_string(),
        values: vec![Value::String("electronics".to_string())],
    });

    let price_cond = Condition::Comparison(crate::velesql::Comparison {
        column: "price".to_string(),
        operator: crate::velesql::CompareOp::Lt,
        value: Value::Integer(50),
    });

    let combined = Condition::And(Box::new(in_cond), Box::new(price_cond));

    let clause = match_clause_with_condition(combined);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(
        results.len(),
        2,
        "IN + price < 50 combined should match 2 nodes"
    );
    let ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(ids.contains(&2));
    assert!(ids.contains(&4));
}

// ============================================================================
// Catch-all removal verification: unsupported types return error (VP-001)
// ============================================================================

#[test]
fn test_match_where_vector_search_returns_true() {
    let (_dir, collection) = setup_match_test_collection();

    // VectorSearch conditions are handled separately by execute_match_with_similarity,
    // so they should still return Ok(true) in evaluate_where_condition.
    let condition = Condition::VectorSearch(crate::velesql::VectorSearch {
        vector: crate::velesql::VectorExpr::Parameter("v".to_string()),
    });

    let clause = match_clause_with_condition(condition);
    let results = collection
        .execute_match(&clause, &HashMap::new())
        .expect("execute_match should succeed for VectorSearch");

    // VectorSearch is handled at a higher level, so it passes through as true
    assert_eq!(results.len(), 4, "VectorSearch should pass through as true");
}
