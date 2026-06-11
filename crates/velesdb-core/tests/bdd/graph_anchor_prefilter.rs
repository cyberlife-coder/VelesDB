//! BDD tests for GraphFirst anchor-id prefiltering (audit 2026-06 follow-up).
//!
//! Contract under test: when a `MATCH (...)` predicate is AND-required by
//! the WHERE clause, retrieval is **exhaustive within the graph matches** —
//! a matching row is found even when it ranks far outside the bounded
//! over-fetch window that the post-filter execution would have fetched.
//! Predicates under `OR` keep the post-filter semantics unchanged.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point, SearchResult};

use super::helpers::create_test_db;

const ROWS: u64 = 30;

/// Parses and executes `sql` against `collection` with a `$q` vector param.
fn run_query(
    db: &Database,
    sql: &str,
    collection: &str,
    vector: Option<&[f32]>,
) -> Vec<SearchResult> {
    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse query");
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String(collection.to_string()),
    );
    if let Some(v) = vector {
        params.insert("q".to_string(), json!(v));
    }
    db.execute_query(&query, &params)
        .expect("test: execute query")
}

/// GIVEN base: 30 points whose similarity to `[1,0,0,0]` strictly decreases
/// with id; ONLY the vector-distant nodes 27 and 28 carry an outgoing
/// `CITES` edge (and a `flagged` payload marker mirroring the edge set).
fn setup_far_anchor_collection(db: &Database) {
    db.create_vector_collection("far", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create far collection");
    let vc = db
        .get_vector_collection("far")
        .expect("test: get far collection");

    let mut points = Vec::new();
    for i in 0..ROWS {
        // Angle from the query grows with i → similarity strictly decreases.
        #[allow(clippy::cast_precision_loss)]
        let off = (i as f32) * 0.2;
        points.push(Point::new(
            i,
            vec![1.0, off, 0.0, 0.0],
            Some(json!({"idx": i, "flagged": i == 27 || i == 28})),
        ));
    }
    points.push(Point::new(
        100,
        vec![-1.0, 0.0, 0.0, 0.5],
        Some(json!({"target": true})),
    ));
    vc.upsert(points).expect("test: upsert");

    for (edge_id, source) in [(900u64, 27u64), (901, 28)] {
        let edge = GraphEdge::new(edge_id, source, 100, "CITES").expect("test: create edge");
        vc.add_edge(edge).expect("test: add edge");
    }
}

/// WHEN the only graph-matching nodes rank ~28th by similarity and LIMIT is 1
/// (over-fetch window = 10) THEN the anchored NEAR fetch still finds the best
/// graph match — retrieval is exhaustive within the anchors, not bounded by
/// the window.
#[test]
fn test_near_match_finds_anchor_beyond_overfetch_window() {
    let (_dir, db) = create_test_db();
    setup_far_anchor_collection(&db);

    let results = run_query(
        &db,
        "SELECT * FROM far AS d WHERE vector NEAR $q AND MATCH (d)-[:CITES]->(x) LIMIT 1",
        "far",
        Some(&[1.0, 0.0, 0.0, 0.0]),
    );

    assert_eq!(results.len(), 1, "the graph match must be found");
    assert_eq!(
        results[0].point.id, 27,
        "node 27 is the most similar of the two anchors"
    );
}

/// Metadata + MATCH without NEAR: the anchored fetch returns exactly the
/// rows satisfying both, without scanning a bounded window.
#[test]
fn test_metadata_and_match_anchored_fetch_is_exact() {
    let (_dir, db) = create_test_db();
    setup_far_anchor_collection(&db);

    let results = run_query(
        &db,
        "SELECT * FROM far AS d WHERE idx >= 28 AND MATCH (d)-[:CITES]->(x) LIMIT 10",
        "far",
        None,
    );

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![28],
        "only node 28 satisfies idx >= 28 AND the edge"
    );
}

/// OR-wrapped MATCH keeps post-filter semantics: rows matching EITHER side
/// are returned (no prefilter may narrow an OR).
#[test]
fn test_or_wrapped_match_keeps_union_semantics() {
    let (_dir, db) = create_test_db();
    setup_far_anchor_collection(&db);

    let results = run_query(
        &db,
        "SELECT * FROM far AS d WHERE idx = 3 OR MATCH (d)-[:CITES]->(x) LIMIT 10",
        "far",
        None,
    );

    let mut ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![3, 27, 28],
        "OR must keep both the metadata match and the graph matches"
    );
}

/// Sparse + MATCH: anchors feed the sparse index's per-id filter, so the
/// anchor docs are returned ranked by sparse score even when non-anchor
/// docs dominate the global sparse ranking at this LIMIT.
#[test]
fn test_sparse_match_returns_anchors_beyond_global_ranking() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("sp", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create sp collection");
    let vc = db
        .get_vector_collection("sp")
        .expect("test: get sp collection");

    // Sparse weight on term 1 decreases with id: docs 1..3 dominate the
    // global sparse ranking; only doc 5 (weakest) has the edge.
    let mut points = Vec::new();
    for i in 1..=5u64 {
        #[allow(clippy::cast_precision_loss)]
        let weight = 10.0 - i as f32;
        let mut sparse = std::collections::BTreeMap::new();
        sparse.insert(
            String::new(),
            velesdb_core::sparse_index::SparseVector::new(vec![(1, weight)]),
        );
        points.push(Point {
            id: i,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(json!({"idx": i})),
            sparse_vectors: Some(sparse),
        });
    }
    vc.upsert(points).expect("test: upsert");
    let edge = GraphEdge::new(900, 5, 1, "CITES").expect("test: create edge");
    vc.add_edge(edge).expect("test: add edge");

    let results = run_query(
        &db,
        "SELECT * FROM sp AS d WHERE vector SPARSE_NEAR {1: 1.0} AND MATCH (d)-[:CITES]->(x) LIMIT 1",
        "sp",
        None,
    );

    assert_eq!(results.len(), 1, "the graph-matching doc must be found");
    assert_eq!(
        results[0].point.id, 5,
        "doc 5 is the only anchor — it must be returned despite ranking last globally"
    );
}

/// NOT-similarity + MATCH: the scan is restricted to the anchor set and the
/// fetch is exact at LIMIT.
#[test]
fn test_not_similarity_match_scans_only_anchors() {
    let (_dir, db) = create_test_db();
    setup_far_anchor_collection(&db);

    // Both anchors (27, 28) are dissimilar enough to pass NOT similarity > 0.99.
    let results = run_query(
        &db,
        "SELECT * FROM far AS d WHERE NOT similarity(vector, $q) > 0.99 \
         AND MATCH (d)-[:CITES]->(x) LIMIT 10",
        "far",
        Some(&[1.0, 0.0, 0.0, 0.0]),
    );

    let mut ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![27, 28], "exactly the anchor rows pass");
}
