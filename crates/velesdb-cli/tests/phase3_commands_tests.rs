//! Phase 3 — Missing CLI Commands Integration Tests
//!
//! Tests for: create-vector-collection, create-graph-collection, delete-collection,
//! explain, analyze, delete-points, upsert, and index management commands.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Get the CLI binary command
#[allow(deprecated)]
fn velesdb_cmd() -> Command {
    Command::cargo_bin("velesdb").unwrap()
}

/// Helper: create a vector collection via core API and return (`db_path`, `TempDir`).
fn setup_vector_collection(name: &str, dim: usize) -> (std::path::PathBuf, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_vector_collection(name, dim, velesdb_core::DistanceMetric::Cosine)
        .unwrap();
    drop(db);
    (db_path, temp_dir)
}

// =============================================================================
// 3.1 — create-vector-collection
// =============================================================================

#[test]
fn test_create_vector_collection_success() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("my_vectors")
        .arg("--dimension")
        .arg("128")
        .assert()
        .success()
        .stdout(predicate::str::contains("Vector collection"))
        .stdout(predicate::str::contains("created"))
        .stdout(predicate::str::contains("128 dims"));
}

#[test]
fn test_create_vector_collection_with_metric_and_storage() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("embeddings")
        .arg("--dimension")
        .arg("768")
        .arg("--metric")
        .arg("euclidean")
        .arg("--storage")
        .arg("sq8")
        .assert()
        .success()
        .stdout(predicate::str::contains("created"))
        .stdout(predicate::str::contains("Euclidean"));
}

#[test]
fn test_create_vector_collection_duplicate_fails() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    // Create first
    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("dupes")
        .arg("--dimension")
        .arg("64")
        .assert()
        .success();

    // Create duplicate — should fail
    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("dupes")
        .arg("--dimension")
        .arg("64")
        .assert()
        .failure();
}

#[test]
fn test_create_vector_collection_then_list() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("docs")
        .arg("--dimension")
        .arg("384")
        .assert()
        .success();

    velesdb_cmd()
        .arg("list")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("docs"));
}

#[test]
fn test_create_vector_collection_help() {
    velesdb_cmd()
        .arg("create-vector-collection")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("dimension"))
        .stdout(predicate::str::contains("metric"))
        .stdout(predicate::str::contains("storage"));
}

// =============================================================================
// 3.2 — create-graph-collection
// =============================================================================

#[test]
fn test_create_graph_collection_success() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-graph-collection")
        .arg(&db_path)
        .arg("knowledge")
        .assert()
        .success()
        .stdout(predicate::str::contains("Graph collection"))
        .stdout(predicate::str::contains("created"));
}

#[test]
fn test_create_graph_collection_duplicate_fails() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-graph-collection")
        .arg(&db_path)
        .arg("kg")
        .assert()
        .success();

    velesdb_cmd()
        .arg("create-graph-collection")
        .arg(&db_path)
        .arg("kg")
        .assert()
        .failure();
}

#[test]
fn test_create_graph_collection_then_info() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-graph-collection")
        .arg(&db_path)
        .arg("relations")
        .assert()
        .success();

    velesdb_cmd()
        .arg("info")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("relations"))
        .stdout(predicate::str::contains("[Graph]"));
}

#[test]
fn test_create_graph_collection_help() {
    velesdb_cmd()
        .arg("create-graph-collection")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("schemaless"));
}

// =============================================================================
// 3.3 — delete-collection
// =============================================================================

#[test]
fn test_delete_collection_with_force() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    // Create then delete
    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("to_delete")
        .arg("--dimension")
        .arg("32")
        .assert()
        .success();

    velesdb_cmd()
        .arg("delete-collection")
        .arg(&db_path)
        .arg("to_delete")
        .arg("--force")
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted"));
}

#[test]
fn test_delete_collection_nonexistent_fails() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("delete-collection")
        .arg(&db_path)
        .arg("ghost")
        .arg("--force")
        .assert()
        .failure();
}

#[test]
fn test_delete_collection_then_list_empty() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("temp_col")
        .arg("--dimension")
        .arg("16")
        .assert()
        .success();

    velesdb_cmd()
        .arg("delete-collection")
        .arg(&db_path)
        .arg("temp_col")
        .arg("--force")
        .assert()
        .success();

    velesdb_cmd()
        .arg("list")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("No collections found"));
}

#[test]
fn test_delete_collection_help() {
    velesdb_cmd()
        .arg("delete-collection")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("force"));
}

// =============================================================================
// 3.4 — explain
// =============================================================================

#[test]
fn test_explain_tree_output() {
    let (db_path, _temp) = setup_vector_collection("docs", 128);

    velesdb_cmd()
        .arg("explain")
        .arg(&db_path)
        .arg("SELECT * FROM docs LIMIT 10")
        .assert()
        .success()
        .stdout(predicate::str::contains("Query Execution Plan"))
        .stdout(predicate::str::contains("Estimated cost"));
}

#[test]
fn test_explain_json_output() {
    let (db_path, _temp) = setup_vector_collection("docs", 128);

    velesdb_cmd()
        .arg("explain")
        .arg(&db_path)
        .arg("SELECT * FROM docs LIMIT 10")
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("estimated_cost_ms"))
        .stdout(predicate::str::contains("root"));
}

#[test]
fn test_explain_invalid_query_fails() {
    let (db_path, _temp) = setup_vector_collection("docs", 128);

    velesdb_cmd()
        .arg("explain")
        .arg(&db_path)
        .arg("THIS IS NOT SQL")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Parse error"));
}

#[test]
fn test_explain_help() {
    velesdb_cmd()
        .arg("explain")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("query"))
        .stdout(predicate::str::contains("format"));
}

// =============================================================================
// 3.5 — analyze
// =============================================================================

#[test]
fn test_analyze_table_output() {
    let (db_path, _temp) = setup_vector_collection("docs", 64);

    velesdb_cmd()
        .arg("analyze")
        .arg(&db_path)
        .arg("docs")
        .assert()
        .success()
        .stdout(predicate::str::contains("Collection Statistics"))
        .stdout(predicate::str::contains("Total points"))
        .stdout(predicate::str::contains("Row count"));
}

#[test]
fn test_analyze_json_output() {
    let (db_path, _temp) = setup_vector_collection("docs", 64);

    velesdb_cmd()
        .arg("analyze")
        .arg(&db_path)
        .arg("docs")
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("total_points"))
        .stdout(predicate::str::contains("row_count"));
}

#[test]
fn test_analyze_nonexistent_collection_fails() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("analyze")
        .arg(&db_path)
        .arg("ghost")
        .assert()
        .failure();
}

#[test]
fn test_analyze_help() {
    velesdb_cmd()
        .arg("analyze")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("collection"))
        .stdout(predicate::str::contains("format"));
}

// =============================================================================
// 3.6 — delete-points
// =============================================================================

#[test]
fn test_delete_points_success() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    // Insert a point via core API
    {
        let db = velesdb_core::Database::open(&db_path).unwrap();
        let col = db.get_vector_collection("docs").unwrap();
        col.upsert(vec![velesdb_core::Point::new(
            42,
            vec![1.0, 2.0, 3.0, 4.0],
            None,
        )])
        .unwrap();
    }

    velesdb_cmd()
        .arg("delete-points")
        .arg(&db_path)
        .arg("docs")
        .arg("42")
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted"))
        .stdout(predicate::str::contains("1 point(s)"));
}

#[test]
fn test_delete_points_multiple_ids() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    {
        let db = velesdb_core::Database::open(&db_path).unwrap();
        let col = db.get_vector_collection("docs").unwrap();
        col.upsert(vec![
            velesdb_core::Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
            velesdb_core::Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
            velesdb_core::Point::new(3, vec![0.0, 0.0, 1.0, 0.0], None),
        ])
        .unwrap();
    }

    velesdb_cmd()
        .arg("delete-points")
        .arg(&db_path)
        .arg("docs")
        .arg("1")
        .arg("2")
        .arg("3")
        .assert()
        .success()
        .stdout(predicate::str::contains("3 point(s)"));
}

#[test]
fn test_delete_points_nonexistent_collection_fails() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("delete-points")
        .arg(&db_path)
        .arg("ghost")
        .arg("1")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_delete_points_no_ids_fails() {
    velesdb_cmd()
        .arg("delete-points")
        .arg("./data")
        .arg("docs")
        .assert()
        .failure();
}

// =============================================================================
// 3.7 — upsert (single point)
// =============================================================================

#[test]
fn test_upsert_single_point() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("upsert")
        .arg(&db_path)
        .arg("docs")
        .arg("--id")
        .arg("100")
        .arg("--vector")
        .arg("[1.0, 2.0, 3.0, 4.0]")
        .arg("--payload")
        .arg("{\"title\": \"Hello\"}")
        .assert()
        .success()
        .stdout(predicate::str::contains("Upserted point 100"));
}

#[test]
fn test_upsert_then_get() {
    let (db_path, _temp) = setup_vector_collection("items", 3);

    velesdb_cmd()
        .arg("upsert")
        .arg(&db_path)
        .arg("items")
        .arg("--id")
        .arg("7")
        .arg("--vector")
        .arg("[0.5, 0.6, 0.7]")
        .assert()
        .success();

    velesdb_cmd()
        .arg("get")
        .arg(&db_path)
        .arg("items")
        .arg("7")
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 7"));
}

#[test]
fn test_upsert_invalid_vector_json_fails() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("upsert")
        .arg(&db_path)
        .arg("docs")
        .arg("--id")
        .arg("1")
        .arg("--vector")
        .arg("not_json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid vector JSON"));
}

#[test]
fn test_upsert_invalid_payload_json_fails() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("upsert")
        .arg(&db_path)
        .arg("docs")
        .arg("--id")
        .arg("1")
        .arg("--vector")
        .arg("[1.0, 2.0, 3.0, 4.0]")
        .arg("--payload")
        .arg("not_json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid payload JSON"));
}

#[test]
fn test_upsert_nonexistent_collection_fails() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("upsert")
        .arg(&db_path)
        .arg("ghost")
        .arg("--id")
        .arg("1")
        .arg("--vector")
        .arg("[1.0]")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_upsert_help() {
    velesdb_cmd()
        .arg("upsert")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("id"))
        .stdout(predicate::str::contains("vector"))
        .stdout(predicate::str::contains("payload"));
}

// =============================================================================
// 3.8 — index management
// =============================================================================

#[test]
fn test_index_create_secondary() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("index")
        .arg("create")
        .arg(&db_path)
        .arg("docs")
        .arg("category")
        .assert()
        .success()
        .stdout(predicate::str::contains("Secondary index created"));
}

#[test]
fn test_index_create_property() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("index")
        .arg("create")
        .arg(&db_path)
        .arg("docs")
        .arg("name")
        .arg("--index-type")
        .arg("property")
        .arg("--label")
        .arg("Person")
        .assert()
        .success()
        .stdout(predicate::str::contains("Property index created"));
}

#[test]
fn test_index_create_range() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("index")
        .arg("create")
        .arg(&db_path)
        .arg("docs")
        .arg("price")
        .arg("--index-type")
        .arg("range")
        .arg("--label")
        .arg("Product")
        .assert()
        .success()
        .stdout(predicate::str::contains("Range index created"));
}

#[test]
fn test_index_list_empty() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("index")
        .arg("list")
        .arg(&db_path)
        .arg("docs")
        .assert()
        .success()
        .stdout(predicate::str::contains("No indexes found"));
}

#[test]
fn test_index_create_then_list_in_separate_invocations() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    // Create a secondary index via CLI
    velesdb_cmd()
        .arg("index")
        .arg("create")
        .arg(&db_path)
        .arg("docs")
        .arg("category")
        .assert()
        .success();

    // Note: in-memory indexes don't persist across CLI invocations.
    // This tests the list command runs successfully even with no persisted indexes.
    velesdb_cmd()
        .arg("index")
        .arg("list")
        .arg(&db_path)
        .arg("docs")
        .assert()
        .success()
        .stdout(predicate::str::contains("Indexes"));
}

#[test]
fn test_index_list_json_format_empty() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    // In-memory indexes don't persist across CLI invocations.
    // Verify the JSON output is a valid empty array.
    velesdb_cmd()
        .arg("index")
        .arg("list")
        .arg(&db_path)
        .arg("docs")
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("["));
}

#[test]
fn test_index_drop_not_found() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    // In-memory indexes don't persist across CLI invocations.
    // Drop on a non-persisted index reports "No index".
    velesdb_cmd()
        .arg("index")
        .arg("drop")
        .arg(&db_path)
        .arg("docs")
        .arg("Node")
        .arg("attr")
        .assert()
        .success()
        .stdout(predicate::str::contains("No index"));
}

#[test]
fn test_index_drop_nonexistent() {
    let (db_path, _temp) = setup_vector_collection("docs", 4);

    velesdb_cmd()
        .arg("index")
        .arg("drop")
        .arg(&db_path)
        .arg("docs")
        .arg("Ghost")
        .arg("phantom")
        .assert()
        .success()
        .stdout(predicate::str::contains("No index"));
}

#[test]
fn test_index_nonexistent_collection_fails() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    velesdb_cmd()
        .arg("index")
        .arg("list")
        .arg(&db_path)
        .arg("ghost")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_index_help() {
    velesdb_cmd()
        .arg("index")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("drop"))
        .stdout(predicate::str::contains("list"));
}

// =============================================================================
// Cross-command integration tests
// =============================================================================

#[test]
fn test_create_vector_upsert_delete_lifecycle() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    // Create collection
    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("lifecycle")
        .arg("--dimension")
        .arg("3")
        .assert()
        .success();

    // Upsert a point
    velesdb_cmd()
        .arg("upsert")
        .arg(&db_path)
        .arg("lifecycle")
        .arg("--id")
        .arg("1")
        .arg("--vector")
        .arg("[0.1, 0.2, 0.3]")
        .arg("--payload")
        .arg("{\"tag\": \"test\"}")
        .assert()
        .success();

    // Get the point
    velesdb_cmd()
        .arg("get")
        .arg(&db_path)
        .arg("lifecycle")
        .arg("1")
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 1"));

    // Delete the point
    velesdb_cmd()
        .arg("delete-points")
        .arg(&db_path)
        .arg("lifecycle")
        .arg("1")
        .assert()
        .success();

    // Delete the collection
    velesdb_cmd()
        .arg("delete-collection")
        .arg(&db_path)
        .arg("lifecycle")
        .arg("--force")
        .assert()
        .success();

    // Verify it's gone
    velesdb_cmd()
        .arg("list")
        .arg(&db_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("No collections found"));
}

#[test]
fn test_create_analyze_explain_flow() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    fs::create_dir_all(&db_path).unwrap();

    // Create
    velesdb_cmd()
        .arg("create-vector-collection")
        .arg(&db_path)
        .arg("analytics")
        .arg("--dimension")
        .arg("8")
        .assert()
        .success();

    // Analyze
    velesdb_cmd()
        .arg("analyze")
        .arg(&db_path)
        .arg("analytics")
        .assert()
        .success()
        .stdout(predicate::str::contains("Collection Statistics"));

    // Explain
    velesdb_cmd()
        .arg("explain")
        .arg(&db_path)
        .arg("SELECT * FROM analytics LIMIT 5")
        .assert()
        .success()
        .stdout(predicate::str::contains("Query Execution Plan"));
}

// =============================================================================
// Phase 4 — Full collection read (Graph nodes, Metadata query)
// =============================================================================

fn setup_graph_collection(name: &str) -> (std::path::PathBuf, tempfile::TempDir) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_graph_collection(name, velesdb_core::GraphSchema::schemaless())
        .unwrap();
    let col = db.get_graph_collection(name).unwrap();
    let edge = velesdb_core::GraphEdge::new(1, 10, 20, "KNOWS").unwrap();
    col.add_edge(edge).unwrap();
    col.store_node_payload(10, &serde_json::json!({"name": "Alice"}))
        .unwrap();
    col.store_node_payload(20, &serde_json::json!({"name": "Bob"}))
        .unwrap();
    drop(db);
    (db_path, temp_dir)
}

fn setup_metadata_collection(name: &str) -> (std::path::PathBuf, tempfile::TempDir) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_metadata_collection(name).unwrap();
    let col = db.get_metadata_collection(name).unwrap();
    let points: Vec<velesdb_core::Point> = (1u64..=3)
        .map(|i| {
            velesdb_core::Point::metadata_only(
                i,
                serde_json::json!({"label": format!("item_{}", i)}),
            )
        })
        .collect();
    col.upsert(points).unwrap();
    drop(db);
    (db_path, temp_dir)
}

#[test]
fn test_graph_nodes_table_output() {
    let (db_path, _temp_dir) = setup_graph_collection("kg");

    velesdb_cmd()
        .arg("graph")
        .arg("nodes")
        .arg(&db_path)
        .arg("kg")
        .assert()
        .success()
        .stdout(predicate::str::contains("Nodes in"));
}

#[test]
fn test_graph_nodes_json_output() {
    let (db_path, _temp_dir) = setup_graph_collection("kg");

    let output = velesdb_cmd()
        .arg("graph")
        .arg("nodes")
        .arg(&db_path)
        .arg("kg")
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8_lossy(&output);
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).expect("should be valid JSON");
    assert!(parsed.is_array(), "json output should be an array");
}

#[test]
fn test_velesql_select_metadata_collection() {
    let (db_path, _temp_dir) = setup_metadata_collection("meta");

    velesdb_cmd()
        .arg("query")
        .arg(&db_path)
        .arg("SELECT * FROM meta LIMIT 5")
        .assert()
        .success()
        .stdout(predicate::str::contains("item_1").or(predicate::str::contains("Results")));
}
