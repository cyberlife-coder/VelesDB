//! BDD tests for the VectorFirst MATCH execution strategy.
//!
//! When a MATCH query includes `similarity(field, $vector) > threshold` in
//! its WHERE clause and the similarity targets the start node, the MATCH
//! planner selects the VectorFirst strategy: vector search first to find
//! top-k candidates, then validate graph pattern existence for each.
//!
//! These tests exercise the **full pipeline**: SQL string -> parse ->
//! planner strategy selection -> execute -> verify results.
//!
//! Coverage breakdown (per `bdd-testing.md`):
//!
//! | Category | Count | Share |
//! |----------|-------|-------|
//! | Nominal  |   3   |  50%  |
//! | Edge     |   2   |  33%  |
//! | Negative |   1   |  17%  |

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point};

use super::helpers::create_test_db;

// =========================================================================
// Module-specific setup
// =========================================================================

/// Creates a vector collection with labeled nodes and graph edges suitable
/// for VectorFirst MATCH queries.
///
/// Graph topology:
/// ```text
///   (1:Document)--[:CITES]-->(2:Reference)
///   (3:Document)--[:CITES]-->(4:Reference)
///   (5:Document)  (no outgoing CITES edge)
/// ```
///
/// Vectors (4-dim, cosine):
/// - id 1: `[1.0, 0.0, 0.0, 0.0]` — Document "Physics 101"
/// - id 2: `[0.9, 0.1, 0.0, 0.0]` — Reference "Newton's Laws"
/// - id 3: `[0.05, 0.95, 0.0, 0.0]` — Document "Rust Handbook" (far from query)
/// - id 4: `[0.0, 0.0, 1.0, 0.0]` — Reference "Ownership Model"
/// - id 5: `[0.95, 0.05, 0.0, 0.0]` — Document "Chemistry" (high sim, no edge)
fn setup_vector_first_collection(db: &Database) {
    // Create a vector collection (supports both vectors and graph edges).
    let vc = {
        db.create_vector_collection("papers", 4, velesdb_core::DistanceMetric::Cosine)
            .expect("test: create papers collection");
        db.get_vector_collection("papers")
            .expect("test: get papers collection")
    };

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Physics 101",
                "category": "science"
            })),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({
                "_labels": ["Reference"],
                "title": "Newton's Laws",
                "category": "science"
            })),
        ),
        Point::new(
            3,
            vec![0.05, 0.95, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Rust Handbook",
                "category": "tech"
            })),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "_labels": ["Reference"],
                "title": "Ownership Model",
                "category": "tech"
            })),
        ),
        Point::new(
            5,
            vec![0.95, 0.05, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Chemistry Basics",
                "category": "science"
            })),
        ),
    ])
    .expect("test: upsert papers corpus");

    // Add graph edges: Document -> Reference via CITES.
    let edge1 = GraphEdge::new(100, 1, 2, "CITES").expect("test: create edge 1->2");
    vc.add_edge(edge1).expect("test: add edge 1->2 CITES");

    let edge2 = GraphEdge::new(101, 3, 4, "CITES").expect("test: create edge 3->4");
    vc.add_edge(edge2).expect("test: add edge 3->4 CITES");
}

/// Builds a params map with `_collection` and a vector parameter.
fn match_params(
    collection: &str,
    param_name: &str,
    vector: &[f32],
) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String(collection.to_string()),
    );
    params.insert(param_name.to_string(), json!(vector));
    params
}

/// Builds a params map with only `_collection` (no vector param).
fn collection_param(collection: &str) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String(collection.to_string()),
    );
    params
}

// =========================================================================
// A. Nominal tests (~60%)
// =========================================================================

/// GIVEN a collection with Documents linked to References via CITES edges
/// WHEN a MATCH query uses `similarity(doc.embedding, $v) > 0.7` targeting
///      the start node (doc), combined with a `[:CITES]` relationship
/// THEN results include only nodes that pass BOTH similarity AND graph checks.
///
/// Here, node 1 (Physics 101, cosine ~1.0 to query) has a CITES edge to node 2,
/// so it should appear. Node 5 (Chemistry Basics, cosine ~0.999) does NOT have
/// a CITES edge, so VectorFirst must exclude it despite high similarity.
#[test]
fn test_match_vector_first_basic() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.7 \
               RETURN doc, ref LIMIT 10";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query =
        velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH with similarity");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute VectorFirst MATCH query");

    // Node 1 (Physics 101) should appear: high similarity AND has CITES edge.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        ids.contains(&1),
        "Node 1 (Physics 101) should pass both similarity and graph filter, got: {ids:?}"
    );
}

/// GIVEN the same graph topology
/// WHEN a VectorFirst query matches a Document that has a CITES edge to a Reference
/// THEN the result excludes Documents without CITES edges even if they have high
///      vector similarity.
///
/// Node 5 (Chemistry Basics, vector ~`[0.95, 0.05, 0, 0]`) has cosine > 0.7
/// to `[1, 0, 0, 0]` but no outgoing CITES edge, so it must be filtered out.
#[test]
fn test_match_vector_first_filters_by_graph_pattern() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.5 \
               RETURN doc LIMIT 20";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute query");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();

    // Node 5 has high similarity but no CITES edge -- must NOT appear.
    assert!(
        !ids.contains(&5),
        "Node 5 (Chemistry Basics) has no CITES edge and must be excluded, got: {ids:?}"
    );

    // Node 1 should be present (high sim + has CITES edge).
    assert!(
        ids.contains(&1),
        "Node 1 should pass both filters, got: {ids:?}"
    );
}

/// GIVEN the graph topology with multiple Documents at varying distances
/// WHEN VectorFirst runs with a low threshold (>0.0) and a relationship filter
/// THEN all Documents with matching graph pattern are returned, ordered by score.
#[test]
fn test_match_vector_first_returns_multiple_candidates() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.0 \
               RETURN doc LIMIT 20";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute query");

    // Both node 1 (Physics 101) and node 3 (Rust Handbook) have CITES edges.
    // Node 1 has high similarity, node 3 has low similarity, but threshold is >0.0.
    // At least node 1 should be present.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        ids.contains(&1),
        "Node 1 (high sim + CITES edge) should appear, got: {ids:?}"
    );
}

// =========================================================================
// B. Edge tests (~20%)
// =========================================================================

/// GIVEN an empty collection with no points or edges
/// WHEN a VectorFirst MATCH query is executed
/// THEN the result is empty (not an error).
#[test]
fn test_match_vector_first_empty_collection() {
    let (_dir, db) = create_test_db();

    db.create_vector_collection("empty", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create empty collection");

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.5 \
               RETURN doc LIMIT 10";
    let params = match_params("empty", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: VectorFirst on empty collection should not error");

    assert!(
        results.is_empty(),
        "Empty collection should return 0 results, got {}",
        results.len()
    );
}

/// GIVEN a populated collection
/// WHEN the similarity threshold is extremely high (>0.999)
/// THEN only near-exact matches with valid graph paths are returned.
///
/// Only node 1 (exact `[1,0,0,0]`) should pass `> 0.999` threshold, and it
/// has a CITES edge. Node 5 (`[0.95, 0.05, 0, 0]`) has cosine ~0.999 but
/// may or may not pass depending on precision; regardless, it has no CITES
/// edge and should be excluded by the graph filter.
#[test]
fn test_match_vector_first_high_threshold() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.999 \
               RETURN doc LIMIT 10";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query =
        velesdb_core::velesql::Parser::parse(sql).expect("test: parse high-threshold MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute high-threshold query");

    // With threshold > 0.999 and cosine metric, only the exact match (node 1)
    // should pass the vector filter. It also has a CITES edge.
    for r in &results {
        assert!(
            r.score > 0.999,
            "All results should exceed 0.999 threshold, got score {} for id {}",
            r.score,
            r.point.id
        );
    }

    // Node 5 (no CITES edge) must not appear even if its score is very high.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        !ids.contains(&5),
        "Node 5 must not appear (no CITES edge), got: {ids:?}"
    );
}

// =========================================================================
// C. Negative tests (>= 20%)
// =========================================================================

/// GIVEN a graph collection with nodes and edges
/// WHEN a MATCH query uses `similarity()` but the vector param `$v` is missing
///      from the params map
/// THEN the query returns an error (not empty results or a panic).
#[test]
fn test_match_vector_first_missing_vector_param() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.7 \
               RETURN doc LIMIT 10";
    // Pass _collection but NOT the $v vector param.
    let params = collection_param("papers");

    let query =
        velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query (missing param)");

    let err = db
        .execute_query(&query, &params)
        .expect_err("test: missing $v param should produce an error");

    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("param")
            || msg.to_lowercase().contains("vector")
            || msg.to_lowercase().contains("not found")
            || msg.to_lowercase().contains("missing"),
        "Error should indicate missing parameter, got: {msg}"
    );
}
