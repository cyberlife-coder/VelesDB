//! BDD-style end-to-end tests for EXPLAIN ANALYZE (Issue #466).
//!
//! Each scenario follows GIVEN (setup data) → WHEN (call `explain_analyze_query`)
//! → THEN (verify `ExplainOutput` fields). Tests exercise the full pipeline:
//! SQL string → `Parser::parse()` → `Database::explain_analyze_query()` → verify.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{velesql::Parser, Database, Point};

use super::helpers::{create_test_db, execute_sql, vector_param};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `docs` vector collection with 5 documents for EXPLAIN ANALYZE tests.
fn setup_vector_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE docs");

    let vc = db
        .get_vector_collection("docs")
        .expect("test: get docs collection");

    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"cat": "a"}))),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"cat": "b"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"cat": "a"}))),
        Point::new(4, vec![0.0, 0.0, 0.0, 1.0], Some(json!({"cat": "b"}))),
        Point::new(5, vec![0.5, 0.5, 0.0, 0.0], Some(json!({"cat": "a"}))),
    ])
    .expect("test: upsert docs");
}

/// Parse a SQL string and call `explain_analyze_query` with the given params.
fn explain_analyze(
    db: &Database,
    sql: &str,
    params: &HashMap<String, serde_json::Value>,
) -> velesdb_core::Result<velesdb_core::velesql::ExplainOutput> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.explain_analyze_query(&query, params)
}

// =========================================================================
// 4.2 — Nominal EXPLAIN ANALYZE scenarios
// =========================================================================

/// GIVEN a collection with vectors
/// WHEN `explain_analyze_query` on SELECT with NEAR
/// THEN `actual_stats` has `actual_rows > 0` and `actual_time_ms > 0.0`
#[test]
fn test_explain_analyze_vector_near_returns_populated_stats() {
    let (_dir, db) = create_test_db();
    setup_vector_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let output = explain_analyze(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 3;",
        &params,
    )
    .expect("test: explain_analyze NEAR");

    let stats = output
        .actual_stats
        .expect("test: actual_stats should be Some");
    assert!(stats.actual_rows > 0, "should return rows");
    assert!(stats.actual_time_ms > 0.0, "execution takes time");
    assert_eq!(stats.loops, 1);
}

/// GIVEN a collection with metadata
/// WHEN `explain_analyze_query` on SELECT with WHERE filter
/// THEN `actual_rows` matches filter result count
#[test]
fn test_explain_analyze_metadata_filter_returns_correct_actual_rows() {
    let (_dir, db) = create_test_db();
    setup_vector_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let sql = "SELECT * FROM docs WHERE cat = 'a' AND vector NEAR $v LIMIT 10;";

    let output = explain_analyze(&db, sql, &params).expect("test: explain_analyze filter");

    // Also run execute_query to get the ground truth count.
    let query = Parser::parse(sql).expect("test: parse");
    let exec_results = db
        .execute_query(&query, &params)
        .expect("test: execute_query");

    let stats = output
        .actual_stats
        .expect("test: actual_stats should be Some");
    assert_eq!(
        stats.actual_rows,
        exec_results.len() as u64,
        "actual_rows must match execute_query count"
    );
}

/// GIVEN a query with multiple plan nodes
/// WHEN `explain_analyze_query`
/// THEN `node_stats` is non-empty and each entry has `actual_time_ms >= 0.0`
#[test]
fn test_explain_analyze_per_node_stats_populated() {
    let (_dir, db) = create_test_db();
    setup_vector_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let output = explain_analyze(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 3;",
        &params,
    )
    .expect("test: explain_analyze per-node");

    assert!(
        !output.node_stats.is_empty(),
        "node_stats should be populated"
    );
    for ns in &output.node_stats {
        assert!(
            ns.actual_time_ms >= 0.0,
            "node {} time must be non-negative",
            ns.node_label
        );
        assert!(ns.loops >= 1, "loops must be >= 1");
    }
}

/// GIVEN a query
/// WHEN both `explain_query` and `explain_analyze_query` are called
/// THEN plans are structurally identical
#[test]
fn test_explain_analyze_plan_matches_explain_query() {
    let (_dir, db) = create_test_db();
    setup_vector_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5;";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let query = Parser::parse(sql).expect("test: parse");

    let explain_plan = db.explain_query(&query).expect("test: explain_query");
    let analyze_output = db
        .explain_analyze_query(&query, &params)
        .expect("test: explain_analyze_query");

    // Compare plan structure (root, index_used, filter_strategy).
    assert_eq!(explain_plan.root, analyze_output.plan.root);
    assert_eq!(explain_plan.index_used, analyze_output.plan.index_used);
    assert_eq!(
        explain_plan.filter_strategy,
        analyze_output.plan.filter_strategy
    );
}

/// GIVEN a query
/// WHEN both `execute_query` and `explain_analyze_query` are called
/// THEN `actual_rows` equals result count
#[test]
fn test_explain_analyze_result_set_matches_execute_query() {
    let (_dir, db) = create_test_db();
    setup_vector_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 4;";
    let params = vector_param(&[0.5, 0.5, 0.0, 0.0]);
    let query = Parser::parse(sql).expect("test: parse");

    let exec_results = db
        .execute_query(&query, &params)
        .expect("test: execute_query");
    let analyze_output = db
        .explain_analyze_query(&query, &params)
        .expect("test: explain_analyze_query");

    let stats = analyze_output
        .actual_stats
        .expect("test: actual_stats should be Some");
    assert_eq!(
        stats.actual_rows,
        exec_results.len() as u64,
        "actual_rows must equal execute_query result count"
    );
}

// =========================================================================
// 4.3 — Edge cases and graph traversal
// =========================================================================

/// GIVEN a query matching nothing
/// WHEN `explain_analyze_query`
/// THEN `actual_rows == 0` and `actual_time_ms > 0.0`
#[test]
fn test_explain_analyze_zero_results_returns_zero_rows_positive_time() {
    let (_dir, db) = create_test_db();
    setup_vector_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let output = explain_analyze(
        &db,
        "SELECT * FROM docs WHERE cat = 'nonexistent' AND vector NEAR $v LIMIT 5;",
        &params,
    )
    .expect("test: explain_analyze zero results");

    let stats = output
        .actual_stats
        .expect("test: actual_stats should be Some");
    assert_eq!(stats.actual_rows, 0, "no rows should match");
    assert!(stats.actual_time_ms > 0.0, "execution still takes time");
}

/// GIVEN an invalid query
/// WHEN `explain_analyze_query`
/// THEN returns `Err` (no panic)
#[test]
fn test_explain_analyze_invalid_query_returns_error() {
    let (_dir, db) = create_test_db();

    // Parser::parse will fail on garbage SQL.
    let parse_result = Parser::parse("NOT A VALID QUERY AT ALL !!!");
    assert!(parse_result.is_err(), "invalid SQL should fail to parse");

    // Also test with a parseable but semantically invalid query
    // (collection does not exist).
    let sql = "SELECT * FROM nonexistent_collection WHERE vector NEAR $v LIMIT 5;";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let result = explain_analyze(&db, sql, &params);
    assert!(
        result.is_err(),
        "query on nonexistent collection should return Err"
    );
}

/// GIVEN a graph collection with edges
/// WHEN `explain_analyze_query` on MATCH
/// THEN `nodes_visited > 0` and `edges_traversed > 0`
#[test]
fn test_explain_analyze_match_graph_returns_nonzero_traversal_counts() {
    let (_dir, db) = create_test_db();

    // Create graph collection and populate nodes + edges.
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE GRAPH COLLECTION");

    let gc = db
        .get_graph_collection("kg")
        .expect("test: get kg collection");

    gc.upsert_node_payload(1, &json!({"_labels": ["Person"], "name": "Alice"}))
        .expect("test: upsert node 1");
    gc.upsert_node_payload(2, &json!({"_labels": ["Person"], "name": "Bob"}))
        .expect("test: upsert node 2");
    gc.upsert_node_payload(3, &json!({"_labels": ["Person"], "name": "Carol"}))
        .expect("test: upsert node 3");

    let e1 = velesdb_core::GraphEdge::new(1, 1, 2, "KNOWS").expect("test: create edge 1->2");
    gc.add_edge(e1).expect("test: add edge 1->2");
    let e2 = velesdb_core::GraphEdge::new(2, 2, 3, "KNOWS").expect("test: create edge 2->3");
    gc.add_edge(e2).expect("test: add edge 2->3");

    // Parse a standalone MATCH query and pass _collection via params.
    let query = Parser::parse("MATCH (a:Person)-[:KNOWS]->(b) RETURN a, b LIMIT 10")
        .expect("test: parse MATCH query");

    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String("kg".to_string()),
    );

    let output = db
        .explain_analyze_query(&query, &params)
        .expect("test: explain_analyze MATCH");

    let stats = output
        .actual_stats
        .expect("test: actual_stats should be Some");
    assert!(stats.actual_rows > 0, "MATCH should return results");
    assert!(
        stats.nodes_visited > 0,
        "graph traversal should visit nodes"
    );
    assert!(
        stats.edges_traversed > 0,
        "graph traversal should traverse edges"
    );
    assert!(stats.actual_time_ms > 0.0, "execution takes time");
}
