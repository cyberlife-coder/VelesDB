//! Fix #492 — Regression tests for MATCH WHERE metadata condition evaluation.
//!
//! Before the fix, the `_ => Ok(true)` catch-all in `evaluate_where_condition`
//! silently ignored `IN`, `NOT IN`, `BETWEEN`, `LIKE`, `ILIKE`, `IS NULL`,
//! and `IS NOT NULL` conditions in MATCH queries, causing all nodes to pass.
//!
//! Each test follows TDD convention: written BEFORE the fix, expected to FAIL
//! until the implementation is corrected.

#![cfg(feature = "persistence")]

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{velesql::Parser, Database, DistanceMetric, Point, VectorCollection};

/// Creates a graph-like collection with diverse node payloads for metadata filtering.
///
/// Nodes:
/// - id=1: category=tech, price=100, name=Alice Wonderland, no optional field
/// - id=2: category=science, price=25, name=Bob Builder, optional field present
/// - id=3: category=tech, price=50, name=Charlie Brown, no optional field
/// - id=4: category=history, price=75, name=alice smith, optional field present
/// - id=5: category=science, price=150, name=Alicia Keys, optional field explicit null
fn setup_collection() -> (TempDir, VectorCollection) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open db");
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create collection");
    let collection = db
        .get_vector_collection("items")
        .expect("test: get collection");

    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 100, "name": "Alice Wonderland"})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "science", "price": 25, "name": "Bob Builder", "optional_field": "present"})),
            ),
            Point::new(
                3,
                vec![0.8, 0.2, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 50, "name": "Charlie Brown"})),
            ),
            Point::new(
                4,
                vec![0.7, 0.3, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "history", "price": 75, "name": "alice smith", "optional_field": "value"})),
            ),
            Point::new(
                5,
                vec![0.6, 0.4, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "science", "price": 150, "name": "Alicia Keys", "optional_field": null})),
            ),
        ])
        .expect("test: upsert items");

    (dir, collection)
}

// ============================================================================
// IN operator
// ============================================================================

#[test]
fn test_match_where_in_filters_nodes() {
    // GIVEN: a collection with nodes having different categories
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with WHERE n.category IN ('tech', 'history')
    let query =
        Parser::parse("MATCH (n:Item) WHERE n.category IN ('tech', 'history') RETURN n LIMIT 10")
            .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: only nodes with category 'tech' or 'history' are returned (ids 1, 3, 4)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![1, 3, 4],
        "IN should filter to tech and history nodes only"
    );
}

#[test]
fn test_match_where_not_in_filters_nodes() {
    // GIVEN: a collection with nodes having different categories
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with WHERE n.category NOT IN ('tech')
    let query = Parser::parse("MATCH (n:Item) WHERE n.category NOT IN ('tech') RETURN n LIMIT 10")
        .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: nodes with category != 'tech' are returned (ids 2, 4, 5)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![2, 4, 5], "NOT IN should exclude tech nodes");
}

// ============================================================================
// BETWEEN operator
// ============================================================================

#[test]
fn test_match_where_between_filters_nodes() {
    // GIVEN: a collection with nodes having different prices
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with WHERE n.price BETWEEN 50 AND 100
    let query = Parser::parse("MATCH (n:Item) WHERE n.price BETWEEN 50 AND 100 RETURN n LIMIT 10")
        .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: nodes with price in [50, 100] are returned (ids 1, 3, 4)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![1, 3, 4],
        "BETWEEN should filter to price 50..=100"
    );
}

// ============================================================================
// LIKE operator
// ============================================================================

#[test]
fn test_match_where_like_filters_nodes() {
    // GIVEN: a collection with nodes having different names
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with WHERE n.name LIKE 'Al%'
    let query = Parser::parse("MATCH (n:Item) WHERE n.name LIKE 'Al%' RETURN n LIMIT 10")
        .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: only names starting with 'Al' (case-sensitive) (ids 1, 5)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![1, 5],
        "LIKE 'Al%' should match Alice Wonderland and Alicia Keys"
    );
}

#[test]
fn test_match_where_ilike_filters_nodes() {
    // GIVEN: a collection with nodes having different names
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with WHERE n.name ILIKE 'al%'
    let query = Parser::parse("MATCH (n:Item) WHERE n.name ILIKE 'al%' RETURN n LIMIT 10")
        .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: case-insensitive, names starting with 'al' (ids 1, 4, 5)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![1, 4, 5],
        "ILIKE 'al%' should match Alice, alice smith, Alicia"
    );
}

// ============================================================================
// IS NULL / IS NOT NULL
// ============================================================================

#[test]
fn test_match_where_is_null_filters_nodes() {
    // GIVEN: a collection where some nodes lack optional_field or have it set to null
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with WHERE n.optional_field IS NULL
    let query = Parser::parse("MATCH (n:Item) WHERE n.optional_field IS NULL RETURN n LIMIT 10")
        .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: nodes where optional_field is absent (1, 3) or explicitly null (5)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![1, 3, 5],
        "IS NULL should match absent and explicit null fields"
    );
}

#[test]
fn test_match_where_is_not_null_filters_nodes() {
    // GIVEN: a collection where some nodes have optional_field present
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with WHERE n.optional_field IS NOT NULL
    let query =
        Parser::parse("MATCH (n:Item) WHERE n.optional_field IS NOT NULL RETURN n LIMIT 10")
            .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: nodes where optional_field is present and non-null (ids 2, 4)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![2, 4],
        "IS NOT NULL should match only non-null present fields"
    );
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_match_where_in_empty_list_returns_nothing() {
    // GIVEN: a collection with items
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with an IN clause containing no values — use comparison fallback
    // Note: IN () with empty parens may not parse, so test with a value that matches nothing
    let query =
        Parser::parse("MATCH (n:Item) WHERE n.category IN ('nonexistent') RETURN n LIMIT 10")
            .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: no results
    assert!(
        results.is_empty(),
        "IN with non-matching value should return nothing"
    );
}

#[test]
fn test_match_where_between_out_of_range_returns_nothing() {
    // GIVEN: a collection with prices from 25 to 150
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with BETWEEN range that excludes all nodes
    let query = Parser::parse("MATCH (n:Item) WHERE n.price BETWEEN 200 AND 300 RETURN n LIMIT 10")
        .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: no results
    assert!(
        results.is_empty(),
        "BETWEEN with out-of-range values should return nothing"
    );
}

#[test]
fn test_match_where_like_no_match_returns_nothing() {
    // GIVEN: a collection with names
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with LIKE pattern that matches no name
    let query = Parser::parse("MATCH (n:Item) WHERE n.name LIKE 'Zzz%' RETURN n LIMIT 10")
        .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: no results
    assert!(
        results.is_empty(),
        "LIKE with non-matching pattern should return nothing"
    );
}

#[test]
fn test_match_where_in_combined_with_comparison() {
    // GIVEN: a collection with items
    let (_dir, collection) = setup_collection();

    // WHEN: MATCH with IN combined with AND comparison
    let query = Parser::parse(
        "MATCH (n:Item) WHERE n.category IN ('tech', 'science') AND n.price > 50 RETURN n LIMIT 10",
    )
    .expect("test: parse");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");

    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");

    // THEN: tech or science nodes with price > 50 (ids 1=tech/100, 5=science/150)
    let mut ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![1, 5],
        "IN + AND comparison should intersect correctly"
    );
}
