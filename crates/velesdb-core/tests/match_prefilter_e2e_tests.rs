#![cfg(feature = "persistence")]
//! E2E BDD tests for MATCH WHERE index prefilter pipeline.
//!
//! Exercises the full path: `VelesQL` parse -> MATCH planner -> index prefilter
//! -> graph traversal -> WHERE evaluation -> results. Verifies that property
//! indexes accelerate MATCH queries WITHOUT changing correctness.
//!
//! Regression coverage for Devin review findings:
//! - GTE/LTE boundary values must be included (not excluded by strict GT/LT)
//! - Compound AND with index narrows correctly

// Reason: test IDs are small literals, safe truncation and precision loss acceptable.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]

use std::collections::{HashMap, HashSet};

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point, VectorCollection};

// =========================================================================
// Helpers
// =========================================================================

/// Creates a `VectorCollection` with 10 nodes having `_labels`, age, city.
/// Ages: 20..29, cities alternate Paris/London.
fn setup_collection() -> (TempDir, VectorCollection) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open db");
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create collection");
    let collection = db
        .get_vector_collection("items")
        .expect("test: get collection");

    let cities = ["Paris", "London"];
    let mut points = Vec::new();
    for i in 0u64..10 {
        let age = 20 + i;
        let city = cities[i as usize % 2];
        points.push(Point::new(
            i,
            vec![1.0 - (i as f32 * 0.05), i as f32 * 0.05, 0.0, 0.0],
            Some(json!({
                "_labels": ["Person"],
                "age": age,
                "city": city,
            })),
        ));
    }
    collection.upsert(points).expect("test: upsert");

    (dir, collection)
}

/// Execute a MATCH query and return node IDs from the results.
fn match_ids(collection: &VectorCollection, sql: &str) -> HashSet<u64> {
    let query = Parser::parse(sql).expect("test: parse VelesQL");
    let match_clause = query.match_clause.as_ref().expect("test: match clause");
    let results = collection
        .execute_match(match_clause, &HashMap::new())
        .expect("test: execute match");
    results.iter().map(|r| r.node_id).collect()
}

// =========================================================================
// GTE boundary inclusion (Devin regression: GTE must not exclude boundary)
// =========================================================================

/// GIVEN: 10 Person nodes with ages 20..29
/// WHEN:  MATCH (n:Person) WHERE n.age >= 25 RETURN n LIMIT 100
/// THEN:  Returns nodes 5,6,7,8,9 (ages 25,26,27,28,29) — age=25 INCLUDED
#[test]
fn test_match_gte_includes_boundary_value() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.age >= 25 RETURN n LIMIT 100",
    );

    // Node 5 has age=25 — MUST be included (GTE is inclusive).
    assert!(
        ids.contains(&5),
        "node 5 (age=25) must be included by >= 25, got: {ids:?}"
    );
    // Nodes 0-4 (ages 20-24) must NOT be included.
    for excluded in 0u64..5 {
        assert!(
            !ids.contains(&excluded),
            "node {excluded} must be excluded by >= 25"
        );
    }
    // Nodes 5-9 (ages 25-29) must all be included.
    for included in 5u64..10 {
        assert!(
            ids.contains(&included),
            "node {included} must be included by >= 25"
        );
    }
}

/// GIVEN: Same setup
/// WHEN:  MATCH (n:Person) WHERE n.age <= 23 RETURN n LIMIT 100
/// THEN:  Returns nodes 0,1,2,3 (ages 20,21,22,23) — age=23 INCLUDED
#[test]
fn test_match_lte_includes_boundary_value() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.age <= 23 RETURN n LIMIT 100",
    );

    assert!(
        ids.contains(&3),
        "node 3 (age=23) must be included by <= 23, got: {ids:?}"
    );
    let expected: HashSet<u64> = [0, 1, 2, 3].into_iter().collect();
    assert_eq!(ids, expected, "ages 20-23 should be included by <= 23");
}

// =========================================================================
// Strict GT excludes boundary
// =========================================================================

/// GIVEN: Same setup
/// WHEN:  MATCH (n:Person) WHERE n.age > 25 RETURN n LIMIT 100
/// THEN:  Returns nodes 6,7,8,9 — age=25 EXCLUDED
#[test]
fn test_match_strict_gt_excludes_boundary() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.age > 25 RETURN n LIMIT 100",
    );

    assert!(
        !ids.contains(&5),
        "node 5 (age=25) must be EXCLUDED by strict > 25"
    );
    let expected: HashSet<u64> = [6, 7, 8, 9].into_iter().collect();
    assert_eq!(ids, expected, "only ages 26-29 for strict > 25");
}

// =========================================================================
// Equality filter
// =========================================================================

/// GIVEN: Same setup
/// WHEN:  MATCH (n:Person) WHERE n.city = 'Paris' RETURN n LIMIT 100
/// THEN:  Returns even-numbered nodes (0,2,4,6,8)
#[test]
fn test_match_equality_filter() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.city = 'Paris' RETURN n LIMIT 100",
    );

    let expected: HashSet<u64> = [0, 2, 4, 6, 8].into_iter().collect();
    assert_eq!(ids, expected, "Paris nodes should be even-numbered");
}

// =========================================================================
// BETWEEN inclusive bounds
// =========================================================================

/// GIVEN: Same setup
/// WHEN:  MATCH (n:Person) WHERE n.age BETWEEN 23 AND 26 RETURN n LIMIT 100
/// THEN:  Returns nodes 3,4,5,6 (ages 23,24,25,26) — both bounds inclusive
#[test]
fn test_match_between_inclusive_bounds() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.age BETWEEN 23 AND 26 RETURN n LIMIT 100",
    );

    let expected: HashSet<u64> = [3, 4, 5, 6].into_iter().collect();
    assert_eq!(
        ids, expected,
        "BETWEEN 23 AND 26 should include ages 23,24,25,26"
    );
}

// =========================================================================
// AND compound condition
// =========================================================================

/// GIVEN: Same setup
/// WHEN:  MATCH (n:Person) WHERE n.age >= 24 AND n.city = 'London' RETURN n LIMIT 100
/// THEN:  Returns London nodes with age >= 24: 5,7,9 (ages 25,27,29)
#[test]
fn test_match_and_compound_condition() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.age >= 24 AND n.city = 'London' RETURN n LIMIT 100",
    );

    // London = odd ids, age >= 24 = ids >= 4. Intersection = 5,7,9.
    let expected: HashSet<u64> = [5, 7, 9].into_iter().collect();
    assert_eq!(
        ids, expected,
        "age >= 24 AND city = London should be nodes 5, 7, 9"
    );
}

// =========================================================================
// Edge: no results
// =========================================================================

/// GIVEN: Same setup (ages 20-29)
/// WHEN:  MATCH (n:Person) WHERE n.age > 100 RETURN n LIMIT 100
/// THEN:  Empty set
#[test]
fn test_match_no_results_above_range() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.age > 100 RETURN n LIMIT 100",
    );

    assert!(ids.is_empty(), "no nodes have age > 100");
}

/// GIVEN: Same setup
/// WHEN:  MATCH (n:Person) WHERE n.city = 'Tokyo' RETURN n LIMIT 100
/// THEN:  Empty set (no Tokyo nodes)
#[test]
fn test_match_equality_no_match() {
    let (_dir, collection) = setup_collection();

    let ids = match_ids(
        &collection,
        "MATCH (n:Person) WHERE n.city = 'Tokyo' RETURN n LIMIT 100",
    );

    assert!(ids.is_empty(), "no nodes have city = Tokyo");
}
