//! Tests for CLI graph commands (EPIC-041).
//!
//! Tests graph subcommand handling and output formatting.

use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    Command::cargo_bin("velesdb").unwrap()
}

#[test]
fn test_graph_help() {
    cmd()
        .args(["graph", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("traverse"))
        .stdout(predicate::str::contains("degree"))
        .stdout(predicate::str::contains("add-edge"));
}

#[test]
fn test_graph_traverse_shows_curl_instructions() {
    let temp = tempfile::tempdir().unwrap();
    cmd()
        .args([
            "graph",
            "traverse",
            temp.path().to_str().unwrap(),
            "test_collection",
            "1",
            "--strategy",
            "bfs",
            "--max-depth",
            "3",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Graph operations require a running VelesDB server",
        ))
        .stdout(predicate::str::contains("curl"));
}

#[test]
fn test_graph_degree_shows_curl_instructions() {
    let temp = tempfile::tempdir().unwrap();
    cmd()
        .args([
            "graph",
            "degree",
            temp.path().to_str().unwrap(),
            "my_collection",
            "42",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("curl"))
        .stdout(predicate::str::contains("degree"));
}

#[test]
fn test_graph_add_edge_shows_curl_instructions() {
    let temp = tempfile::tempdir().unwrap();
    cmd()
        .args([
            "graph",
            "add-edge",
            temp.path().to_str().unwrap(),
            "edges_collection",
            "100",   // edge id
            "1",     // source
            "2",     // target
            "KNOWS", // label
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("curl"))
        .stdout(predicate::str::contains("edges"));
}

#[test]
fn test_graph_traverse_with_rel_types() {
    let temp = tempfile::tempdir().unwrap();
    cmd()
        .args([
            "graph",
            "traverse",
            temp.path().to_str().unwrap(),
            "graph_col",
            "5",
            "--rel-types",
            "KNOWS,FOLLOWS",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("traverse"));
}

#[test]
fn test_graph_traverse_dfs_strategy() {
    let temp = tempfile::tempdir().unwrap();
    cmd()
        .args([
            "graph",
            "traverse",
            temp.path().to_str().unwrap(),
            "dfs_test",
            "10",
            "--strategy",
            "dfs",
            "--limit",
            "50",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("dfs"));
}
