//! BDD tests for hybrid SELECT queries combining vector NEAR, graph MATCH
//! predicates, and scalar filters in a single WHERE clause.
//!
//! Regression coverage for the production panic where graph predicates forced
//! `execution_limit = MAX_LIMIT` (100k) and the downstream oversampling clamp
//! hit `f64::clamp` with `min > max` ("triple hybrid" showcase query), and for
//! the late runtime-only anchor-alias check that is now a validation error.
//!
//! All tests exercise the full pipeline: SQL string -> parse -> validate ->
//! execute -> verify.

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, vector_param};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Creates an "articles" collection mixing vectors, payloads, and graph edges.
///
/// Graph topology (CITES):
/// ```text
///   (1)--[:CITES]-->(2)
///   (3)--[:CITES]-->(2)
///   (4)--[:CITES]-->(2)
///   (5)--[:CITES]-->(2)
///   (2) has no outgoing edge
/// ```
///
/// Vectors (4-dim, cosine), query is `[1, 0, 0, 0]`:
///
/// | id | vector            | category | has outgoing CITES |
/// |----|-------------------|----------|--------------------|
/// | 1  | `[1.0,0,0,0]`     | science  | yes                |
/// | 2  | `[0.9,0.1,0,0]`   | science  | no                 |
/// | 3  | `[0.85,0.15,0,0]` | science  | yes                |
/// | 4  | `[0.8,0.2,0,0]`   | tech     | yes                |
/// | 5  | `[0.75,0.25,0,0]` | science  | yes                |
fn setup_articles_with_edges(db: &Database) {
    db.create_vector_collection("articles", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create articles collection");
    let vc = db
        .get_vector_collection("articles")
        .expect("test: get articles collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Quantum"})),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Chemistry"})),
        ),
        Point::new(
            3,
            vec![0.85, 0.15, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Biology"})),
        ),
        Point::new(
            4,
            vec![0.8, 0.2, 0.0, 0.0],
            Some(json!({"category": "tech", "title": "Rust"})),
        ),
        Point::new(
            5,
            vec![0.75, 0.25, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Geology"})),
        ),
    ])
    .expect("test: upsert articles corpus");

    for (edge_id, source) in [(100u64, 1u64), (101, 3), (102, 4), (103, 5)] {
        let edge = GraphEdge::new(edge_id, source, 2, "CITES").expect("test: create edge");
        vc.add_edge(edge).expect("test: add CITES edge");
    }
}

// =========================================================================
// A. Nominal: triple hybrid (NEAR + graph MATCH + scalar) must not panic
// =========================================================================

/// GIVEN articles with vectors, categories, and CITES edges
/// WHEN running the showcase triple-hybrid query
///      `SELECT a.*, similarity() ... WHERE vector NEAR $v
///       AND MATCH (a)-[:CITES]->(r) AND category = 'science'
///       ORDER BY similarity() DESC LIMIT 2`
/// THEN it returns exactly the top-2 similarity-ordered nodes that satisfy
///      BOTH the graph predicate and the scalar filter (no panic, LIMIT kept).
#[test]
fn test_near_graph_match_scalar_orderby_similarity_respects_limit() {
    let (_dir, db) = create_test_db();
    setup_articles_with_edges(&db);

    let sql = "SELECT a.*, similarity() FROM articles AS a \
               WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(r) AND category = 'science' \
               ORDER BY similarity() DESC LIMIT 2";
    let results = execute_sql_with_params(&db, sql, &vector_param(&[1.0, 0.0, 0.0, 0.0]))
        .expect("triple hybrid NEAR + MATCH + scalar must not fail");

    // Candidates passing graph + scalar filters: 1, 3, 5 (2 lacks an outgoing
    // edge, 4 is tech). LIMIT 2 keeps the two most similar: 1 then 3.
    assert_eq!(results.len(), 2, "LIMIT 2 must be respected");
    assert_eq!(results[0].point.id, 1, "highest similarity first");
    assert_eq!(results[1].point.id, 3, "second highest similarity");
    assert!(
        results[0].score >= results[1].score,
        "ORDER BY similarity() DESC must hold"
    );
}

/// GIVEN the same hybrid corpus
/// WHEN the graph predicate anchors on the FROM table without an alias
///      (`FROM articles WHERE MATCH (a)-[:CITES]->(r)`)
/// THEN the query still executes (anchor check only applies when FROM/JOIN
///      aliases are declared) and returns only nodes with outgoing edges.
#[test]
fn test_graph_match_without_from_alias_still_executes() {
    let (_dir, db) = create_test_db();
    setup_articles_with_edges(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE MATCH (a)-[:CITES]->(r) LIMIT 10",
    )
    .expect("MATCH anchor on unaliased FROM must keep working");

    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        [1u64, 3, 4, 5].into_iter().collect(),
        "only nodes with an outgoing CITES edge match"
    );
}

// =========================================================================
// B. Negative: anchor alias mismatch is a clear validation error
// =========================================================================

/// GIVEN an `agent_memory` collection with vectors and RELATES_TO edges
/// WHEN running the mission query verbatim, whose MATCH anchors on `ctx`
///      while the FROM alias is `memory`
/// THEN the query is rejected with a clear, actionable error naming the
///      mismatched anchor alias BEFORE any execution (no panic, no results).
#[test]
fn test_mission_query_anchor_alias_mismatch_is_clear_error() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("agent_memory", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create agent_memory");
    let vc = db
        .get_vector_collection("agent_memory")
        .expect("test: get agent_memory");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"kind": "fact"}))),
        Point::new(2, vec![0.9, 0.1, 0.0, 0.0], Some(json!({"kind": "fact"}))),
    ])
    .expect("test: upsert agent_memory");
    let edge = GraphEdge::new(200, 1, 2, "RELATES_TO").expect("test: create edge");
    vc.add_edge(edge).expect("test: add RELATES_TO edge");

    let sql = "SELECT memory.*, similarity() FROM agent_memory AS memory \
               WHERE vector NEAR $v AND MATCH (ctx)-[:RELATES_TO]->(fact) AND kind = 'fact' \
               ORDER BY similarity() DESC LIMIT 10";
    let err = execute_sql_with_params(&db, sql, &vector_param(&[1.0, 0.0, 0.0, 0.0]))
        .expect_err("anchor alias 'ctx' does not match FROM alias 'memory'");

    let msg = err.to_string();
    assert!(
        msg.contains("ctx"),
        "error must name the mismatched anchor alias, got: {msg}"
    );
    assert!(
        msg.contains("memory"),
        "error must list the declared FROM/JOIN aliases, got: {msg}"
    );
}
