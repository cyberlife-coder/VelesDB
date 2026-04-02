//! BDD tests for REPL graph commands (GIVEN → WHEN → THEN).
//!
//! Tests the REPL command dispatcher for all `.graph` subcommands.
//! Each test creates a real database, executes REPL commands, and verifies
//! the underlying state via core API calls.

use tempfile::TempDir;
use velesdb_core::collection::graph::GraphSchema;
use velesdb_core::{Database, GraphCollection, GraphEdge};

use crate::repl_commands::CommandResult;
use crate::repl_graph_cmds::cmd_graph;

// =========================================================================
// Helpers
// =========================================================================

fn setup_db() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_graph_collection("kg", GraphSchema::schemaless())
        .expect("test: create graph collection");
    (dir, db)
}

fn populate(db: &Database) {
    let col = db
        .get_graph_collection("kg")
        .expect("test: get graph collection");
    for (id, src, tgt, lbl) in [
        (100, 1, 2, "KNOWS"),
        (101, 2, 3, "KNOWS"),
        (102, 3, 4, "KNOWS"),
        (103, 2, 5, "WROTE"),
    ] {
        col.add_edge(GraphEdge::new(id, src, tgt, lbl).expect("valid edge"))
            .expect("test: add edge");
    }
}

fn graph_col(db: &Database) -> GraphCollection {
    db.get_graph_collection("kg")
        .expect("test: get graph collection")
}

/// Assert that a CommandResult is Continue (not Error or Quit).
fn assert_continue(result: &CommandResult) {
    match result {
        CommandResult::Continue => {}
        CommandResult::Error(e) => panic!("Expected Continue, got Error: {e}"),
        CommandResult::Quit => panic!("Expected Continue, got Quit"),
    }
}

/// Assert that a CommandResult is Error.
fn assert_error(result: &CommandResult) {
    assert!(
        matches!(result, CommandResult::Error(_)),
        "Expected Error, got {:?}",
        match result {
            CommandResult::Continue => "Continue",
            CommandResult::Quit => "Quit",
            CommandResult::Error(e) => e.as_str(),
        }
    );
}

// =========================================================================
// A. .graph remove-edge — Nominal
// =========================================================================

#[test]
fn test_repl_remove_edge_existing_removes_it() {
    // GIVEN: a graph with 4 edges
    let (_dir, db) = setup_db();
    populate(&db);
    assert_eq!(graph_col(&db).edge_count(), 4);

    // WHEN: .graph remove-edge kg 100
    let parts: Vec<&str> = vec![".graph", "remove-edge", "kg", "100"];
    let result = cmd_graph(&db, &parts);

    // THEN: success, edge count is 3
    assert_continue(&result);
    assert_eq!(graph_col(&db).edge_count(), 3);
}

#[test]
fn test_repl_remove_edge_nonexistent_no_error() {
    // GIVEN: a graph with 4 edges
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph remove-edge kg 999
    let parts: Vec<&str> = vec![".graph", "remove-edge", "kg", "999"];
    let result = cmd_graph(&db, &parts);

    // THEN: no error, count unchanged
    assert_continue(&result);
    assert_eq!(graph_col(&db).edge_count(), 4);
}

// =========================================================================
// B. .graph remove-edge — Edge cases
// =========================================================================

#[test]
fn test_repl_remove_edge_twice_same_id() {
    // GIVEN: a graph with edge 100
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: remove edge 100 twice
    let parts: Vec<&str> = vec![".graph", "remove-edge", "kg", "100"];
    assert_continue(&cmd_graph(&db, &parts));
    let result = cmd_graph(&db, &parts);

    // THEN: second removal is a no-op, no error
    assert_continue(&result);
    assert_eq!(graph_col(&db).edge_count(), 3);
}

// =========================================================================
// C. .graph remove-edge — Negative
// =========================================================================

#[test]
fn test_repl_remove_edge_missing_args_shows_usage() {
    // GIVEN: a database
    let (_dir, db) = setup_db();

    // WHEN: .graph remove-edge (no collection or edge_id)
    let parts: Vec<&str> = vec![".graph", "remove-edge"];
    let result = cmd_graph(&db, &parts);

    // THEN: shows usage (Continue, not Error)
    assert_continue(&result);
}

#[test]
fn test_repl_remove_edge_invalid_id_returns_error() {
    // GIVEN: a database
    let (_dir, db) = setup_db();

    // WHEN: .graph remove-edge kg not_a_number
    let parts: Vec<&str> = vec![".graph", "remove-edge", "kg", "not_a_number"];
    let result = cmd_graph(&db, &parts);

    // THEN: error
    assert_error(&result);
}

#[test]
fn test_repl_remove_edge_nonexistent_collection_returns_error() {
    // GIVEN: a database with no "ghost" collection
    let (_dir, db) = setup_db();

    // WHEN: .graph remove-edge ghost 1
    let parts: Vec<&str> = vec![".graph", "remove-edge", "ghost", "1"];
    let result = cmd_graph(&db, &parts);

    // THEN: error
    assert_error(&result);
}

// =========================================================================
// D. .graph count — Nominal
// =========================================================================

#[test]
fn test_repl_count_populated_graph() {
    // GIVEN: a graph with 4 edges
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph count kg
    let parts: Vec<&str> = vec![".graph", "count", "kg"];
    let result = cmd_graph(&db, &parts);

    // THEN: success (output goes to stdout, we verify via core)
    assert_continue(&result);
    assert_eq!(graph_col(&db).edge_count(), 4);
}

#[test]
fn test_repl_count_empty_graph() {
    // GIVEN: an empty graph
    let (_dir, db) = setup_db();

    // WHEN: .graph count kg
    let parts: Vec<&str> = vec![".graph", "count", "kg"];
    let result = cmd_graph(&db, &parts);

    // THEN: success, zero counts
    assert_continue(&result);
    assert_eq!(graph_col(&db).edge_count(), 0);
}

// =========================================================================
// E. .graph count — Negative
// =========================================================================

#[test]
fn test_repl_count_nonexistent_collection_returns_error() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph", "count", "ghost"];
    let result = cmd_graph(&db, &parts);

    assert_error(&result);
}

#[test]
fn test_repl_count_missing_args_shows_usage() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph", "count"];
    let result = cmd_graph(&db, &parts);

    assert_continue(&result); // shows usage
}

// =========================================================================
// F. .graph search — Negative (no embeddings)
// =========================================================================

#[test]
fn test_repl_search_graph_without_embeddings_returns_error() {
    // GIVEN: a graph collection WITHOUT embeddings
    let (_dir, db) = setup_db();

    // WHEN: .graph search kg [1.0,0.0,0.0,0.0]
    let parts: Vec<&str> = vec![".graph", "search", "kg", "[1.0,0.0,0.0,0.0]"];
    let result = cmd_graph(&db, &parts);

    // THEN: error about no embeddings
    assert_error(&result);
}

#[test]
fn test_repl_search_invalid_vector_json_returns_error() {
    // GIVEN: a graph collection
    let (_dir, db) = setup_db();

    // WHEN: .graph search kg not_json
    let parts: Vec<&str> = vec![".graph", "search", "kg", "not_json"];
    let result = cmd_graph(&db, &parts);

    // THEN: error about invalid JSON
    assert_error(&result);
}

#[test]
fn test_repl_search_nonexistent_collection_returns_error() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph", "search", "ghost", "[1.0]"];
    let result = cmd_graph(&db, &parts);

    assert_error(&result);
}

#[test]
fn test_repl_search_missing_args_shows_usage() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph", "search"];
    let result = cmd_graph(&db, &parts);

    assert_continue(&result); // shows usage
}

// =========================================================================
// G. .graph store-payload — Nominal
// =========================================================================

#[test]
fn test_repl_store_payload_creates_payload() {
    // GIVEN: a graph collection
    let (_dir, db) = setup_db();

    // WHEN: .graph store-payload kg 42 {"name":"Alice"}
    let parts: Vec<&str> = vec![".graph", "store-payload", "kg", "42", r#"{"name":"Alice"}"#];
    let result = cmd_graph(&db, &parts);

    // THEN: payload stored
    assert_continue(&result);
    let payload = graph_col(&db).get_node_payload(42).unwrap().unwrap();
    assert_eq!(payload["name"], "Alice");
}

#[test]
fn test_repl_store_payload_overwrites() {
    // GIVEN: a node with payload
    let (_dir, db) = setup_db();
    let parts1: Vec<&str> = vec![".graph", "store-payload", "kg", "1", r#"{"v":1}"#];
    assert_continue(&cmd_graph(&db, &parts1));

    // WHEN: overwrite
    let parts2: Vec<&str> = vec![".graph", "store-payload", "kg", "1", r#"{"v":2}"#];
    let result = cmd_graph(&db, &parts2);

    // THEN: new value
    assert_continue(&result);
    let payload = graph_col(&db).get_node_payload(1).unwrap().unwrap();
    assert_eq!(payload["v"], 2);
}

// =========================================================================
// H. .graph store-payload — Negative
// =========================================================================

#[test]
fn test_repl_store_payload_invalid_json_returns_error() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph", "store-payload", "kg", "1", "not_json"];
    let result = cmd_graph(&db, &parts);

    assert_error(&result);
}

// =========================================================================
// I. .graph get-payload — Nominal
// =========================================================================

#[test]
fn test_repl_get_payload_existing_node() {
    // GIVEN: a node with payload
    let (_dir, db) = setup_db();
    graph_col(&db)
        .upsert_node_payload(10, &serde_json::json!({"role": "admin"}))
        .unwrap();

    // WHEN: .graph get-payload kg 10
    let parts: Vec<&str> = vec![".graph", "get-payload", "kg", "10"];
    let result = cmd_graph(&db, &parts);

    // THEN: success (prints to stdout)
    assert_continue(&result);
}

#[test]
fn test_repl_get_payload_nonexistent_node_prints_null() {
    // GIVEN: an empty graph
    let (_dir, db) = setup_db();

    // WHEN: .graph get-payload kg 999
    let parts: Vec<&str> = vec![".graph", "get-payload", "kg", "999"];
    let result = cmd_graph(&db, &parts);

    // THEN: success (prints "null")
    assert_continue(&result);
}

// =========================================================================
// J. .graph nodes — Nominal
// =========================================================================

#[test]
fn test_repl_nodes_with_payloads() {
    // GIVEN: a graph with stored payloads
    let (_dir, db) = setup_db();
    let col = graph_col(&db);
    col.upsert_node_payload(1, &serde_json::json!({"name": "A"}))
        .unwrap();
    col.upsert_node_payload(2, &serde_json::json!({"name": "B"}))
        .unwrap();

    // WHEN: .graph nodes kg
    let parts: Vec<&str> = vec![".graph", "nodes", "kg"];
    let result = cmd_graph(&db, &parts);

    // THEN: success
    assert_continue(&result);
}

#[test]
fn test_repl_nodes_empty_graph() {
    // GIVEN: an empty graph
    let (_dir, db) = setup_db();

    // WHEN: .graph nodes kg
    let parts: Vec<&str> = vec![".graph", "nodes", "kg"];
    let result = cmd_graph(&db, &parts);

    // THEN: success (shows empty page)
    assert_continue(&result);
}

// =========================================================================
// K. .graph nodes — Negative
// =========================================================================

#[test]
fn test_repl_nodes_nonexistent_collection_returns_error() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph", "nodes", "ghost"];
    let result = cmd_graph(&db, &parts);

    assert_error(&result);
}

// =========================================================================
// L. .graph add-edge — Nominal (existing command, verify still works)
// =========================================================================

#[test]
fn test_repl_add_edge_creates_edge() {
    // GIVEN: an empty graph
    let (_dir, db) = setup_db();

    // WHEN: .graph add-edge kg 1 10 20 KNOWS
    let parts: Vec<&str> = vec![".graph", "add-edge", "kg", "1", "10", "20", "KNOWS"];
    let result = cmd_graph(&db, &parts);

    // THEN: edge exists
    assert_continue(&result);
    assert_eq!(graph_col(&db).edge_count(), 1);
    let edges = graph_col(&db).get_edges(Some("KNOWS"));
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].source(), 10);
    assert_eq!(edges[0].target(), 20);
}

// =========================================================================
// M. .graph edges — Nominal
// =========================================================================

#[test]
fn test_repl_edges_with_label_filter() {
    // GIVEN: a graph with mixed labels
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph edges kg --label KNOWS
    let parts: Vec<&str> = vec![".graph", "edges", "kg", "--label", "KNOWS"];
    let result = cmd_graph(&db, &parts);

    // THEN: success
    assert_continue(&result);
    // Verify via core: 3 KNOWS edges
    let edges = graph_col(&db).get_edges(Some("KNOWS"));
    assert_eq!(edges.len(), 3);
}

// =========================================================================
// N. .graph degree — Nominal
// =========================================================================

#[test]
fn test_repl_degree_shows_correct_values() {
    // GIVEN: a graph where node 2 has in=1, out=2
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph degree kg 2
    let parts: Vec<&str> = vec![".graph", "degree", "kg", "2"];
    let result = cmd_graph(&db, &parts);

    // THEN: success
    assert_continue(&result);
    let (in_deg, out_deg) = graph_col(&db).node_degree(2);
    assert_eq!(in_deg, 1);
    assert_eq!(out_deg, 2);
}

// =========================================================================
// O. .graph traverse — Nominal
// =========================================================================

#[test]
fn test_repl_traverse_bfs_default() {
    // GIVEN: a populated graph
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph traverse kg 1
    let parts: Vec<&str> = vec![".graph", "traverse", "kg", "1"];
    let result = cmd_graph(&db, &parts);

    // THEN: success
    assert_continue(&result);
}

#[test]
fn test_repl_traverse_dfs_with_depth() {
    // GIVEN: a populated graph
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph traverse kg 1 --algo dfs --depth 2
    let parts: Vec<&str> = vec![
        ".graph", "traverse", "kg", "1", "--algo", "dfs", "--depth", "2",
    ];
    let result = cmd_graph(&db, &parts);

    // THEN: success
    assert_continue(&result);
}

// =========================================================================
// P. .graph neighbors — Nominal
// =========================================================================

#[test]
fn test_repl_neighbors_outgoing() {
    // GIVEN: a graph where node 2 has 2 outgoing edges
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph neighbors kg 2
    let parts: Vec<&str> = vec![".graph", "neighbors", "kg", "2"];
    let result = cmd_graph(&db, &parts);

    // THEN: success
    assert_continue(&result);
    assert_eq!(graph_col(&db).get_outgoing(2).len(), 2);
}

#[test]
fn test_repl_neighbors_incoming() {
    // GIVEN: a graph where node 3 has 1 incoming edge
    let (_dir, db) = setup_db();
    populate(&db);

    // WHEN: .graph neighbors kg 3 --direction in
    let parts: Vec<&str> = vec![".graph", "neighbors", "kg", "3", "--direction", "in"];
    let result = cmd_graph(&db, &parts);

    // THEN: success
    assert_continue(&result);
    assert_eq!(graph_col(&db).get_incoming(3).len(), 1);
}

// =========================================================================
// Q. Unknown subcommand — Negative
// =========================================================================

#[test]
fn test_repl_graph_unknown_subcommand_returns_error() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph", "foobar"];
    let result = cmd_graph(&db, &parts);

    assert_error(&result);
}

#[test]
fn test_repl_graph_no_subcommand_shows_help() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".graph"];
    let result = cmd_graph(&db, &parts);

    // Shows help, returns Continue
    assert_continue(&result);
}

// =========================================================================
// R. .upsert — Nominal (data command, tested here for completeness)
// =========================================================================

#[test]
fn test_repl_upsert_creates_point() {
    // GIVEN: a database with a vector collection
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_vector_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create vector collection");

    // WHEN: .upsert docs 1 [1.0,0.0,0.0,0.0] {"title":"hello"}
    let parts: Vec<&str> = vec![
        ".upsert",
        "docs",
        "1",
        "[1.0,0.0,0.0,0.0]",
        r#"{"title":"hello"}"#,
    ];
    let result = crate::repl_data_cmds::cmd_upsert(&db, &parts);

    // THEN: success, point exists
    assert_continue(&result);
    let col = db.get_vector_collection("docs").expect("get col");
    let points = col.get(&[1]);
    assert!(points[0].is_some());
    assert_eq!(points[0].as_ref().unwrap().id, 1);
}

#[test]
fn test_repl_upsert_without_payload() {
    // GIVEN: a vector collection
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_vector_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create vector collection");

    // WHEN: .upsert docs 2 [0.0,1.0,0.0,0.0] (no payload)
    let parts: Vec<&str> = vec![".upsert", "docs", "2", "[0.0,1.0,0.0,0.0]"];
    let result = crate::repl_data_cmds::cmd_upsert(&db, &parts);

    // THEN: success
    assert_continue(&result);
    let col = db.get_vector_collection("docs").expect("get col");
    let points = col.get(&[2]);
    assert!(points[0].is_some());
}

#[test]
fn test_repl_upsert_invalid_vector_returns_error() {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_vector_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create vector collection");

    let parts: Vec<&str> = vec![".upsert", "docs", "1", "not_json"];
    let result = crate::repl_data_cmds::cmd_upsert(&db, &parts);

    assert_error(&result);
}

#[test]
fn test_repl_upsert_nonexistent_collection_returns_error() {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");

    let parts: Vec<&str> = vec![".upsert", "ghost", "1", "[1.0]"];
    let result = crate::repl_data_cmds::cmd_upsert(&db, &parts);

    assert_error(&result);
}

#[test]
fn test_repl_upsert_missing_args_shows_usage() {
    let (_dir, db) = setup_db();

    let parts: Vec<&str> = vec![".upsert"];
    let result = crate::repl_data_cmds::cmd_upsert(&db, &parts);

    assert_continue(&result); // shows usage
}
