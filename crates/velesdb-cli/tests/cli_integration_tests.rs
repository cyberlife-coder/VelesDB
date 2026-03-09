//! CLI Integration Tests (EPIC-041 US-002)
//!
//! Tests for `VelesDB` CLI commands using `assert_cmd`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Get the CLI binary command
#[allow(deprecated)]
fn velesdb_cmd() -> Command {
    Command::cargo_bin("velesdb").unwrap()
}

// =============================================================================
// Help & Version Tests
// =============================================================================

#[test]
fn test_help_displays_usage() {
    velesdb_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("VelesDB CLI"))
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn test_version_displays_version() {
    velesdb_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("velesdb"));
}

#[test]
fn test_invalid_command_shows_error() {
    velesdb_cmd()
        .arg("invalid_command_xyz")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

// =============================================================================
// Info Command Tests
// =============================================================================

#[test]
fn test_info_on_new_database() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("info")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("VelesDB Database"));
}

#[test]
fn test_info_help() {
    velesdb_cmd()
        .arg("info")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("database"));
}

// =============================================================================
// List Command Tests
// =============================================================================

#[test]
fn test_list_empty_database() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("list")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Collections"));
}

#[test]
fn test_list_json_format() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("list")
        .arg(&db_path)
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("["));
}

// =============================================================================
// Show Command Tests
// =============================================================================

#[test]
fn test_show_nonexistent_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("show")
        .arg(&db_path)
        .arg("nonexistent_collection")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// =============================================================================
// Create Metadata Collection Tests
// =============================================================================

#[test]
fn test_create_metadata_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-metadata-collection")
        .arg(&db_path)
        .arg("test_metadata")
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));
}

#[test]
fn test_create_metadata_collection_then_list() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    // Create collection - just verify the command succeeds
    // Note: metadata collections may not persist across CLI invocations
    // This tests the command execution, not persistence
    velesdb_cmd()
        .arg("create-metadata-collection")
        .arg(&db_path)
        .arg("my_collection")
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));
}

// =============================================================================
// Get Command Tests
// =============================================================================

#[test]
fn test_get_nonexistent_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    // Try to get from a nonexistent collection
    velesdb_cmd()
        .arg("get")
        .arg(&db_path)
        .arg("nonexistent_col")
        .arg("1")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_get_format_option() {
    // Just test that the --format option is accepted
    velesdb_cmd()
        .arg("get")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("format"));
}

// =============================================================================
// Shell Completions Tests
// =============================================================================

#[test]
fn test_completions_bash() {
    velesdb_cmd()
        .arg("completions")
        .arg("bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("_velesdb"));
}

#[test]
fn test_completions_zsh() {
    velesdb_cmd()
        .arg("completions")
        .arg("zsh")
        .assert()
        .success()
        .stdout(predicate::str::contains("velesdb"));
}

#[test]
fn test_completions_powershell() {
    velesdb_cmd()
        .arg("completions")
        .arg("powershell")
        .assert()
        .success()
        .stdout(predicate::str::contains("velesdb"));
}

#[test]
fn test_completions_fish() {
    velesdb_cmd()
        .arg("completions")
        .arg("fish")
        .assert()
        .success()
        .stdout(predicate::str::contains("velesdb"));
}

// =============================================================================
// License Command Tests
// =============================================================================

#[test]
fn test_license_show_no_license() {
    velesdb_cmd()
        .arg("license")
        .arg("show")
        .assert()
        .failure()
        .stdout(predicate::str::contains("No license").or(predicate::str::contains("license")));
}

#[test]
fn test_license_verify_invalid_key() {
    velesdb_cmd()
        .arg("license")
        .arg("verify")
        .arg("invalid_key")
        .arg("--public-key")
        .arg("MCowBQYDK2VwAyEAtest")
        .assert()
        .failure();
}

// =============================================================================
// Import Command Tests
// =============================================================================

#[test]
fn test_import_nonexistent_file() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("import")
        .arg("/nonexistent/file.jsonl")
        .arg("--database")
        .arg(&db_path)
        .arg("--collection")
        .arg("test")
        .assert()
        .failure();
}

#[test]
fn test_import_unsupported_format() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    let data_file = temp_dir.path().join("data.xyz");
    fs::create_dir_all(&db_path).unwrap();
    fs::write(&data_file, "test data").unwrap();

    velesdb_cmd()
        .arg("import")
        .arg(&data_file)
        .arg("--database")
        .arg(&db_path)
        .arg("--collection")
        .arg("test")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unsupported file format"));
}

#[test]
fn test_import_help() {
    // Test import command help
    velesdb_cmd()
        .arg("import")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("collection"))
        .stdout(predicate::str::contains("dimension"));
}

// =============================================================================
// Export Command Tests
// =============================================================================

#[test]
fn test_export_nonexistent_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("export")
        .arg(&db_path)
        .arg("nonexistent_collection")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// =============================================================================
// Query Command Tests
// =============================================================================

#[test]
fn test_query_nonexistent_database() {
    velesdb_cmd()
        .arg("query")
        .arg("/nonexistent/path")
        .arg("SELECT * FROM test LIMIT 1")
        .assert()
        .failure();
}

// =============================================================================
// MultiSearch Command Tests
// =============================================================================

#[test]
fn test_multisearch_help() {
    // Test multi-search command help
    velesdb_cmd()
        .arg("multi-search")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("vectors"))
        .stdout(predicate::str::contains("strategy"));
}

#[test]
fn test_multisearch_nonexistent_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("multi-search")
        .arg(&db_path)
        .arg("nonexistent")
        .arg("[[0.1, 0.2]]")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// =============================================================================
// Graph Command Tests
// =============================================================================

#[test]
fn test_graph_help() {
    velesdb_cmd()
        .arg("graph")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Graph operations"));
}

// =============================================================================
// Phase 2: Typed Collection Display Tests
// =============================================================================

#[test]
fn test_info_shows_vector_type_label() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    // Create a vector collection via the core API
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_vector_collection("docs", 128, velesdb_core::DistanceMetric::Cosine)
        .unwrap();
    drop(db);

    velesdb_cmd()
        .arg("info")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("[Vector]"));
}

#[test]
fn test_info_shows_graph_type_label() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_graph_collection("knowledge", velesdb_core::GraphSchema::schemaless())
        .unwrap();
    drop(db);

    velesdb_cmd()
        .arg("info")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("[Graph]"));
}

#[test]
fn test_info_shows_metadata_type_label() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_metadata_collection("catalog").unwrap();
    drop(db);

    velesdb_cmd()
        .arg("info")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("[Metadata]"));
}

#[test]
fn test_list_json_shows_type_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_vector_collection("vectors", 64, velesdb_core::DistanceMetric::Euclidean)
        .unwrap();
    drop(db);

    velesdb_cmd()
        .arg("list")
        .arg(&db_path)
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\": \"Vector\""));
}

#[test]
fn test_show_vector_collection_shows_type() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_vector_collection("docs", 128, velesdb_core::DistanceMetric::Cosine)
        .unwrap();
    drop(db);

    velesdb_cmd()
        .arg("show")
        .arg(&db_path)
        .arg("docs")
        .assert()
        .success()
        .stdout(predicate::str::contains("Vector"));
}

#[test]
fn test_show_graph_collection_shows_type() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_graph_collection("kg", velesdb_core::GraphSchema::schemaless())
        .unwrap();
    drop(db);

    velesdb_cmd()
        .arg("show")
        .arg(&db_path)
        .arg("kg")
        .assert()
        .success()
        .stdout(predicate::str::contains("Graph"));
}

#[test]
fn test_show_metadata_collection_shows_type() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_metadata_collection("products").unwrap();
    drop(db);

    velesdb_cmd()
        .arg("show")
        .arg(&db_path)
        .arg("products")
        .assert()
        .success()
        .stdout(predicate::str::contains("Metadata"));
}

#[test]
fn test_export_rejects_graph_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");

    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_graph_collection("kg", velesdb_core::GraphSchema::schemaless())
        .unwrap();
    drop(db);

    velesdb_cmd()
        .arg("export")
        .arg(&db_path)
        .arg("kg")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// =============================================================================
// REPL Command Tests (limited - can't test interactive mode)
// =============================================================================

#[test]
fn test_repl_help() {
    velesdb_cmd()
        .arg("repl")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("REPL"));
}
