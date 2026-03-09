//! Complete E2E Test Suite for `VelesDB` CLI
//!
//! EPIC-060: Comprehensive E2E tests for CLI commands.
//! Tests all CLI subcommands and their options.

#![allow(deprecated)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Get the CLI binary command
fn cli() -> Command {
    Command::cargo_bin("velesdb").unwrap()
}

/// Create a temporary database directory
fn temp_db_dir() -> TempDir {
    TempDir::new().expect("Failed to create temp dir")
}

// ============================================================================
// Info & List Commands E2E Tests
// ============================================================================

mod info_commands {
    use super::*;

    #[test]
    fn test_info_empty_database() {
        let temp = temp_db_dir();

        cli()
            .arg("info")
            .arg(temp.path())
            .assert()
            .success()
            .stdout(predicate::str::contains("Collections"));
    }

    #[test]
    fn test_list_collections_json() {
        let temp = temp_db_dir();

        cli()
            .arg("list")
            .arg(temp.path())
            .arg("--format")
            .arg("json")
            .assert()
            .success()
            .stdout(predicate::str::starts_with("["));
    }

    #[test]
    fn test_list_collections_table() {
        let temp = temp_db_dir();

        cli()
            .arg("list")
            .arg(temp.path())
            .arg("--format")
            .arg("table")
            .assert()
            .success();
    }
}

// ============================================================================
// Query Commands E2E Tests
// ============================================================================

mod query_commands {
    use super::*;

    #[test]
    fn test_query_invalid_syntax() {
        let temp = temp_db_dir();

        cli()
            .arg("query")
            .arg(temp.path())
            .arg("INVALID QUERY SYNTAX")
            .assert()
            .failure();
    }

    #[test]
    fn test_query_select_with_limit() {
        let temp = temp_db_dir();

        // Collection does not exist — command should fail gracefully
        cli()
            .arg("query")
            .arg(temp.path())
            .arg("SELECT * FROM nonexistent LIMIT 10")
            .assert()
            .failure();
    }
}

// ============================================================================
// Multi-Search Commands E2E Tests
// ============================================================================

mod multi_search_commands {
    use super::*;

    #[test]
    fn test_multi_search_rrf_strategy() {
        let temp = temp_db_dir();

        // Collection does not exist — command should fail gracefully
        cli()
            .arg("multi-search")
            .arg(temp.path())
            .arg("test_collection")
            .arg("[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]")
            .arg("--strategy")
            .arg("rrf")
            .arg("--rrf-k")
            .arg("60")
            .arg("-k")
            .arg("10")
            .assert()
            .failure();
    }

    #[test]
    fn test_multi_search_average_strategy() {
        let temp = temp_db_dir();

        cli()
            .arg("multi-search")
            .arg(temp.path())
            .arg("test_collection")
            .arg("[[1.0, 0.0], [0.0, 1.0]]")
            .arg("--strategy")
            .arg("average")
            .assert()
            .failure();
    }

    #[test]
    fn test_multi_search_json_output() {
        let temp = temp_db_dir();

        cli()
            .arg("multi-search")
            .arg(temp.path())
            .arg("test_collection")
            .arg("[[1.0, 0.0]]")
            .arg("--format")
            .arg("json")
            .assert()
            .failure();
    }
}

// ============================================================================
// Graph Commands E2E Tests
// ============================================================================

mod graph_commands {
    use super::*;

    /// Helper: create a graph collection via CLI.
    fn create_graph(temp: &TempDir, name: &str) {
        cli()
            .args([
                "create-graph-collection",
                temp.path().to_str().unwrap(),
                name,
            ])
            .assert()
            .success();
    }

    #[test]
    fn test_graph_traverse_bfs() {
        let temp = temp_db_dir();
        create_graph(&temp, "g");

        // Add an edge so traversal has something to find
        cli()
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

        cli()
            .args([
                "graph",
                "traverse",
                temp.path().to_str().unwrap(),
                "g",
                "10",
                "--algorithm",
                "bfs",
                "--max-depth",
                "3",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Traversal Results"));
    }

    #[test]
    fn test_graph_traverse_dfs() {
        let temp = temp_db_dir();
        create_graph(&temp, "g");

        cli()
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

        cli()
            .args([
                "graph",
                "traverse",
                temp.path().to_str().unwrap(),
                "g",
                "10",
                "--algorithm",
                "dfs",
                "--max-depth",
                "5",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("DFS"));
    }

    #[test]
    fn test_graph_degree() {
        let temp = temp_db_dir();
        create_graph(&temp, "g");

        cli()
            .args(["graph", "degree", temp.path().to_str().unwrap(), "g", "1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("In-degree:"))
            .stdout(predicate::str::contains("Out-degree:"));
    }

    #[test]
    fn test_graph_add_edge() {
        let temp = temp_db_dir();
        create_graph(&temp, "g");

        cli()
            .args([
                "graph",
                "add-edge",
                temp.path().to_str().unwrap(),
                "g",
                "1",
                "100",
                "200",
                "related",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Edge 1 added"));
    }
}

// ============================================================================
// Import/Export Commands E2E Tests
// ============================================================================

mod import_export_commands {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_import_jsonl() {
        let temp = temp_db_dir();
        let jsonl_file = temp.path().join("data.jsonl");

        // Create test JSONL file
        let mut file = fs::File::create(&jsonl_file).unwrap();
        writeln!(file, r#"{{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}}"#).unwrap();
        writeln!(file, r#"{{"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]}}"#).unwrap();

        cli()
            .arg("import")
            .arg(&jsonl_file)
            .arg("--database")
            .arg(temp.path())
            .arg("--collection")
            .arg("imported")
            .arg("--metric")
            .arg("cosine")
            .assert()
            .success()
            .stdout(predicate::str::contains("Import Summary"));
    }

    #[test]
    fn test_export_collection() {
        let temp = temp_db_dir();
        let output_file = temp.path().join("export.json");

        // Collection does not exist — export should fail gracefully
        cli()
            .arg("export")
            .arg(temp.path())
            .arg("test_collection")
            .arg("--output")
            .arg(&output_file)
            .assert()
            .failure();
    }
}

// ============================================================================
// License Commands E2E Tests
// ============================================================================

mod license_commands {
    use super::*;

    #[test]
    fn test_license_show_no_license() {
        cli()
            .arg("license")
            .arg("show")
            .assert()
            .failure()
            .stdout(predicate::str::contains("No license"));
    }

    #[test]
    fn test_license_verify_invalid() {
        cli()
            .arg("license")
            .arg("verify")
            .arg("invalid_key")
            .arg("--public-key")
            .arg("invalid_public_key")
            .assert()
            .failure();
    }
}

// ============================================================================
// Completions Command E2E Tests
// ============================================================================

mod completions_commands {
    use super::*;

    #[test]
    fn test_completions_bash() {
        cli()
            .arg("completions")
            .arg("bash")
            .assert()
            .success()
            .stdout(predicate::str::contains("complete"));
    }

    #[test]
    fn test_completions_zsh() {
        cli()
            .arg("completions")
            .arg("zsh")
            .assert()
            .success()
            .stdout(predicate::str::contains("compdef"));
    }

    #[test]
    fn test_completions_powershell() {
        cli()
            .arg("completions")
            .arg("powershell")
            .assert()
            .success();
    }
}

// ============================================================================
// REPL Commands E2E Tests
// ============================================================================

mod repl_commands {
    use super::*;

    #[test]
    fn test_repl_help() {
        cli()
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("VelesDB CLI"));
    }

    #[test]
    fn test_repl_version() {
        cli().arg("--version").assert().success();
    }
}

// ============================================================================
// Get Command E2E Tests
// ============================================================================

mod get_commands {
    use super::*;

    #[test]
    fn test_get_point_json() {
        let temp = temp_db_dir();

        cli()
            .arg("get")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1")
            .arg("--format")
            .arg("json")
            .assert()
            .failure();
    }

    #[test]
    fn test_get_point_table() {
        let temp = temp_db_dir();

        cli()
            .arg("get")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1")
            .arg("--format")
            .arg("table")
            .assert()
            .failure();
    }
}

// ============================================================================
// Show Command E2E Tests
// ============================================================================

mod show_commands {
    use super::*;

    #[test]
    fn test_show_collection_json() {
        let temp = temp_db_dir();

        cli()
            .arg("show")
            .arg(temp.path())
            .arg("test_collection")
            .arg("--format")
            .arg("json")
            .assert()
            .failure();
    }

    #[test]
    fn test_show_collection_with_samples() {
        let temp = temp_db_dir();

        cli()
            .arg("show")
            .arg(temp.path())
            .arg("test_collection")
            .arg("--samples")
            .arg("5")
            .assert()
            .failure();
    }
}
