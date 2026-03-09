//! Tests for CLI graph commands (Phase 4 — direct core calls).
//!
//! Each test creates a real graph collection, performs operations via CLI,
//! and asserts on the output.

use assert_cmd::Command;
use predicates::prelude::*;

#[allow(deprecated)]
fn cmd() -> Command {
    Command::cargo_bin("velesdb").expect("velesdb binary not found")
}

/// Helper: create a graph collection in a temp directory via CLI.
fn create_graph_collection(dir: &std::path::Path, name: &str) {
    cmd()
        .args(["create-graph-collection", dir.to_str().unwrap(), name])
        .assert()
        .success();
}

// =========================================================================
// Help / subcommand listing
// =========================================================================

#[test]
fn test_graph_help_lists_all_subcommands() {
    cmd()
        .args(["graph", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("add-edge"))
        .stdout(predicate::str::contains("get-edges"))
        .stdout(predicate::str::contains("degree"))
        .stdout(predicate::str::contains("traverse"))
        .stdout(predicate::str::contains("neighbors"))
        .stdout(predicate::str::contains("store-payload"))
        .stdout(predicate::str::contains("get-payload"));
}

// =========================================================================
// add-edge
// =========================================================================

#[test]
fn test_graph_add_edge_success() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "kg");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "kg",
            "1",     // edge id
            "100",   // source
            "200",   // target
            "KNOWS", // label
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Edge 1 added"))
        .stdout(predicate::str::contains("KNOWS"));
}

#[test]
fn test_graph_add_edge_collection_not_found() {
    let temp = tempfile::tempdir().unwrap();
    // No collection created

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "nonexistent",
            "1",
            "100",
            "200",
            "KNOWS",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_graph_add_edge_empty_label_rejected() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "kg");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "kg",
            "1",
            "100",
            "200",
            "   ", // whitespace-only label
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("label"));
}

// =========================================================================
// get-edges
// =========================================================================

#[test]
fn test_graph_get_edges_empty() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args(["graph", "get-edges", temp.path().to_str().unwrap(), "g"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No edges found"));
}

#[test]
fn test_graph_get_edges_after_add() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    // Add two edges
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "10",
            "20",
            "LIKES",
        ])
        .assert()
        .success();
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "2",
            "20",
            "30",
            "FOLLOWS",
        ])
        .assert()
        .success();

    // List all edges
    cmd()
        .args(["graph", "get-edges", temp.path().to_str().unwrap(), "g"])
        .assert()
        .success()
        .stdout(predicate::str::contains("LIKES"))
        .stdout(predicate::str::contains("FOLLOWS"))
        .stdout(predicate::str::contains("Total: 2 edge(s)"));
}

#[test]
fn test_graph_get_edges_filter_by_label() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "10",
            "20",
            "LIKES",
        ])
        .assert()
        .success();
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "2",
            "20",
            "30",
            "FOLLOWS",
        ])
        .assert()
        .success();

    cmd()
        .args([
            "graph",
            "get-edges",
            temp.path().to_str().unwrap(),
            "g",
            "--label",
            "LIKES",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("LIKES"))
        .stdout(predicate::str::contains("Total: 1 edge(s)"));
}

#[test]
fn test_graph_get_edges_json_format() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "10",
            "20",
            "REL",
        ])
        .assert()
        .success();

    let output = cmd()
        .args([
            "graph",
            "get-edges",
            temp.path().to_str().unwrap(),
            "g",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = parsed.as_array().expect("JSON array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[0]["source"], 10);
    assert_eq!(arr[0]["target"], 20);
    assert_eq!(arr[0]["label"], "REL");
}

// =========================================================================
// degree
// =========================================================================

#[test]
fn test_graph_degree_zero_for_isolated_node() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args(["graph", "degree", temp.path().to_str().unwrap(), "g", "999"])
        .assert()
        .success()
        .stdout(predicate::str::contains("In-degree:"))
        .stdout(predicate::str::contains("Out-degree:"))
        .stdout(predicate::str::contains("Total:"));
}

#[test]
fn test_graph_degree_after_edges() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    // 10 -> 20 and 10 -> 30
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "10",
            "20",
            "A",
        ])
        .assert()
        .success();
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "2",
            "10",
            "30",
            "B",
        ])
        .assert()
        .success();

    let output = cmd()
        .args([
            "graph",
            "degree",
            temp.path().to_str().unwrap(),
            "g",
            "10",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["out_degree"], 2);
    assert_eq!(parsed["in_degree"], 0);
    assert_eq!(parsed["total_degree"], 2);
}

// =========================================================================
// traverse
// =========================================================================

#[test]
fn test_graph_traverse_bfs_empty() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args(["graph", "traverse", temp.path().to_str().unwrap(), "g", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));
}

#[test]
fn test_graph_traverse_bfs_finds_reachable_nodes() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    // Chain: 1 -> 2 -> 3
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "10",
            "1",
            "2",
            "NEXT",
        ])
        .assert()
        .success();
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "11",
            "2",
            "3",
            "NEXT",
        ])
        .assert()
        .success();

    cmd()
        .args([
            "graph",
            "traverse",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "--algorithm",
            "bfs",
            "--max-depth",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total: 2 result(s)"));
}

#[test]
fn test_graph_traverse_dfs() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "10",
            "1",
            "2",
            "X",
        ])
        .assert()
        .success();

    cmd()
        .args([
            "graph",
            "traverse",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "--algorithm",
            "dfs",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("DFS"))
        .stdout(predicate::str::contains("Total: 1 result(s)"));
}

#[test]
fn test_graph_traverse_json_format() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "10",
            "1",
            "2",
            "REL",
        ])
        .assert()
        .success();

    let output = cmd()
        .args([
            "graph",
            "traverse",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = parsed.as_array().expect("JSON array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["target_id"], 2);
    assert_eq!(arr[0]["depth"], 1);
}

#[test]
fn test_graph_traverse_with_rel_types() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    // 1 --KNOWS--> 2, 1 --FOLLOWS--> 3
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "10",
            "1",
            "2",
            "KNOWS",
        ])
        .assert()
        .success();
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "11",
            "1",
            "3",
            "FOLLOWS",
        ])
        .assert()
        .success();

    // Only traverse KNOWS edges
    cmd()
        .args([
            "graph",
            "traverse",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "--rel-types",
            "KNOWS",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total: 1 result(s)"));
}

// =========================================================================
// neighbors
// =========================================================================

#[test]
fn test_graph_neighbors_outgoing() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "10",
            "20",
            "A",
        ])
        .assert()
        .success();

    cmd()
        .args([
            "graph",
            "neighbors",
            temp.path().to_str().unwrap(),
            "g",
            "10",
            "--direction",
            "out",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total: 1 edge(s)"));
}

#[test]
fn test_graph_neighbors_incoming() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "10",
            "20",
            "A",
        ])
        .assert()
        .success();

    // Node 20 has incoming from 10
    cmd()
        .args([
            "graph",
            "neighbors",
            temp.path().to_str().unwrap(),
            "g",
            "20",
            "--direction",
            "in",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total: 1 edge(s)"));
}

#[test]
fn test_graph_neighbors_both() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    // 10 -> 20 and 30 -> 20
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "10",
            "20",
            "A",
        ])
        .assert()
        .success();
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "g",
            "2",
            "30",
            "20",
            "B",
        ])
        .assert()
        .success();

    // Node 20: 0 outgoing + 2 incoming
    cmd()
        .args([
            "graph",
            "neighbors",
            temp.path().to_str().unwrap(),
            "g",
            "20",
            "--direction",
            "both",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total: 2 edge(s)"));
}

// =========================================================================
// store-payload / get-payload
// =========================================================================

#[test]
fn test_graph_store_and_get_payload() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "store-payload",
            temp.path().to_str().unwrap(),
            "g",
            "42",
            r#"{"name":"Alice","age":30}"#,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Payload stored on node 42"));

    let output = cmd()
        .args([
            "graph",
            "get-payload",
            temp.path().to_str().unwrap(),
            "g",
            "42",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["name"], "Alice");
    assert_eq!(parsed["age"], 30);
}

#[test]
fn test_graph_get_payload_returns_null_when_empty() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "get-payload",
            temp.path().to_str().unwrap(),
            "g",
            "999",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("null"));
}

#[test]
fn test_graph_store_payload_invalid_json() {
    let temp = tempfile::tempdir().unwrap();
    create_graph_collection(temp.path(), "g");

    cmd()
        .args([
            "graph",
            "store-payload",
            temp.path().to_str().unwrap(),
            "g",
            "1",
            "not-json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid JSON"));
}
