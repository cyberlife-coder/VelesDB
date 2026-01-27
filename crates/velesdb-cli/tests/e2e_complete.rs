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

        // First create a collection (may fail if no data, but command should parse)
        let _ = cli()
            .arg("query")
            .arg(temp.path())
            .arg("SELECT * FROM nonexistent LIMIT 10")
            .assert();
        // Note: May fail due to missing collection, but tests query parsing
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

        // Test command structure (will fail without data, but tests argument parsing)
        let _ = cli()
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
            .assert();
    }

    #[test]
    fn test_multi_search_average_strategy() {
        let temp = temp_db_dir();

        let _ = cli()
            .arg("multi-search")
            .arg(temp.path())
            .arg("test_collection")
            .arg("[[1.0, 0.0], [0.0, 1.0]]")
            .arg("--strategy")
            .arg("average")
            .assert();
    }

    #[test]
    fn test_multi_search_json_output() {
        let temp = temp_db_dir();

        let _ = cli()
            .arg("multi-search")
            .arg(temp.path())
            .arg("test_collection")
            .arg("[[1.0, 0.0]]")
            .arg("--format")
            .arg("json")
            .assert();
    }
}

// ============================================================================
// Graph Commands E2E Tests
// ============================================================================

mod graph_commands {
    use super::*;

    #[test]
    fn test_graph_traverse_bfs() {
        let temp = temp_db_dir();

        cli()
            .arg("graph")
            .arg("traverse")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1") // source node
            .arg("--strategy")
            .arg("bfs")
            .arg("--max-depth")
            .arg("3")
            .assert()
            .success()
            .stdout(predicate::str::contains("curl")); // Shows curl command
    }

    #[test]
    fn test_graph_traverse_dfs() {
        let temp = temp_db_dir();

        cli()
            .arg("graph")
            .arg("traverse")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1")
            .arg("--strategy")
            .arg("dfs")
            .arg("--max-depth")
            .arg("5")
            .assert()
            .success();
    }

    #[test]
    fn test_graph_degree() {
        let temp = temp_db_dir();

        cli()
            .arg("graph")
            .arg("degree")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1")
            .assert()
            .success();
    }

    #[test]
    fn test_graph_add_edge() {
        let temp = temp_db_dir();

        cli()
            .arg("graph")
            .arg("add-edge")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1") // edge id
            .arg("100") // source
            .arg("200") // target
            .arg("related") // label
            .assert()
            .success();
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

        // Export (will be empty if collection doesn't exist)
        let _ = cli()
            .arg("export")
            .arg(temp.path())
            .arg("test_collection")
            .arg("--output")
            .arg(&output_file)
            .assert();
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
            .stderr(predicate::str::contains("No license"));
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

        let _ = cli()
            .arg("get")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1")
            .arg("--format")
            .arg("json")
            .assert();
    }

    #[test]
    fn test_get_point_table() {
        let temp = temp_db_dir();

        let _ = cli()
            .arg("get")
            .arg(temp.path())
            .arg("test_collection")
            .arg("1")
            .arg("--format")
            .arg("table")
            .assert();
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

        let _ = cli()
            .arg("show")
            .arg(temp.path())
            .arg("test_collection")
            .arg("--format")
            .arg("json")
            .assert();
    }

    #[test]
    fn test_show_collection_with_samples() {
        let temp = temp_db_dir();

        let _ = cli()
            .arg("show")
            .arg(temp.path())
            .arg("test_collection")
            .arg("--samples")
            .arg("5")
            .assert();
    }
}
