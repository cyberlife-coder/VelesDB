//! E2E tests for REPL commands against the `VelesDB` core.
//!
//! These tests validate that REPL dot-commands (`.sample`, `.browse`, `.nodes`,
//! `.export`, `.count`, `.stats`) work end-to-end for all three collection types:
//! Vector, Graph, and Metadata. Commands are piped via stdin to the `repl`
//! subcommand; the test asserts on the captured stdout.
//!
//! [`repl_run`] is a thin helper that pipes commands + `.quit` via stdin.

#![allow(deprecated)]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, GraphEdge, GraphSchema, Point};

// ============================================================================
// Helpers
// ============================================================================

fn velesdb_cmd() -> Command {
    Command::cargo_bin("velesdb").unwrap()
}

/// Runs a sequence of REPL commands (one per line) against `db_path` and returns
/// the asserted output. Each command string should NOT include a trailing newline.
///
/// `.quit` is appended automatically.
fn repl_run(db_path: &std::path::Path, commands: &[&str]) -> assert_cmd::assert::Assert {
    let mut input = commands.join("\n");
    input.push_str("\n.quit\n");

    velesdb_cmd()
        .arg("repl")
        .arg(db_path)
        .write_stdin(input)
        .assert()
        .success()
}

/// Create a vector collection with a few points and return `(db_path, TempDir)`.
fn setup_vector(name: &str, dim: usize) -> (std::path::PathBuf, TempDir) {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("db");
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_vector_collection(name, dim, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_vector_collection(name).unwrap();
    for i in 1u64..=5 {
        #[allow(clippy::cast_precision_loss)]
        let vec: Vec<f32> = (0..dim).map(|j| (i as f32 + j as f32) / 100.0).collect();
        col.upsert(vec![Point {
            id: i,
            vector: vec,
            payload: Some(serde_json::json!({"label": format!("vec_{}", i)})),
            sparse_vectors: None,
        }])
        .unwrap();
    }
    drop(db);
    (db_path, temp)
}

/// Create a graph collection with edges and node payloads.
fn setup_graph(name: &str) -> (std::path::PathBuf, TempDir) {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("db");
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_graph_collection(name, GraphSchema::schemaless())
        .unwrap();
    let col = db.get_graph_collection(name).unwrap();
    for i in 1u64..=3 {
        let edge = GraphEdge::new(i, i * 10, i * 10 + 1, "LINKS").unwrap();
        col.add_edge(edge).unwrap();
        col.store_node_payload(
            i * 10,
            &serde_json::json!({"name": format!("node_{}", i * 10)}),
        )
        .unwrap();
    }
    col.flush().unwrap();
    drop(db);
    (db_path, temp)
}

/// Create a metadata-only collection with a few items.
fn setup_metadata(name: &str) -> (std::path::PathBuf, TempDir) {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("db");
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_metadata_collection(name).unwrap();
    let col = db.get_metadata_collection(name).unwrap();
    let points: Vec<Point> = (1u64..=5)
        .map(|i| Point::metadata_only(i, serde_json::json!({"title": format!("item_{}", i)})))
        .collect();
    col.upsert(points).unwrap();
    drop(db);
    (db_path, temp)
}

// ============================================================================
// .collections — lists all collection types
// ============================================================================

#[test]
fn test_repl_collections_lists_all_types() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("db");
    let db = velesdb_core::Database::open(&db_path).unwrap();
    db.create_vector_collection("vecs", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_graph_collection("kg", GraphSchema::schemaless())
        .unwrap();
    db.create_metadata_collection("meta").unwrap();
    drop(db);

    repl_run(&db_path, &[".collections"])
        .stdout(predicate::str::contains("vecs"))
        .stdout(predicate::str::contains("kg"))
        .stdout(predicate::str::contains("meta"));
}

// ============================================================================
// .count
// ============================================================================

#[test]
fn test_repl_count_vector_collection() {
    let (db_path, _temp) = setup_vector("vecs", 8);
    repl_run(&db_path, &[".count vecs"]).stdout(predicate::str::contains("5"));
}

#[test]
fn test_repl_count_metadata_collection() {
    let (db_path, _temp) = setup_metadata("meta");
    repl_run(&db_path, &[".count meta"]).stdout(predicate::str::contains("5"));
}

// ============================================================================
// .sample — works for Vector, Graph, Metadata
// ============================================================================

#[test]
fn test_repl_sample_vector_shows_vector_column() {
    let (db_path, _temp) = setup_vector("vecs", 4);
    repl_run(&db_path, &[".sample vecs 3"])
        .stdout(predicate::str::contains("sample(s) from"))
        .stdout(predicate::str::contains("vector"));
}

#[test]
fn test_repl_sample_graph_no_vector_column() {
    let (db_path, _temp) = setup_graph("kg");
    let assert = repl_run(&db_path, &[".sample kg 5"]);
    // Should show nodes but NOT a "vector" column header
    assert
        .stdout(predicate::str::contains("sample(s) from").or(predicate::str::contains("node")))
        .stdout(predicate::str::contains("vector").not());
}

#[test]
fn test_repl_sample_metadata_no_vector_column() {
    let (db_path, _temp) = setup_metadata("meta");
    repl_run(&db_path, &[".sample meta 3"])
        .stdout(predicate::str::contains("sample(s) from"))
        .stdout(predicate::str::contains("vector").not());
}

#[test]
fn test_repl_sample_unknown_collection_returns_error() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("db");
    velesdb_core::Database::open(&db_path).unwrap();

    repl_run(&db_path, &[".sample nonexistent"])
        .stdout(predicate::str::contains("not found").or(predicate::str::contains("Error")));
}

// ============================================================================
// .browse — paginated display for all types
// ============================================================================

#[test]
fn test_repl_browse_vector_page1() {
    let (db_path, _temp) = setup_vector("vecs", 4);
    repl_run(&db_path, &[".browse vecs 1"])
        .stdout(predicate::str::contains("Page 1"))
        .stdout(predicate::str::contains("total records"));
}

#[test]
fn test_repl_browse_graph_shows_unique_nodes() {
    let (db_path, _temp) = setup_graph("kg");
    repl_run(&db_path, &[".browse kg 1"])
        .stdout(predicate::str::contains("Page 1"))
        .stdout(predicate::str::contains("unique nodes"));
}

#[test]
fn test_repl_browse_metadata_no_vector_column() {
    let (db_path, _temp) = setup_metadata("meta");
    repl_run(&db_path, &[".browse meta 1"])
        .stdout(predicate::str::contains("Page 1"))
        .stdout(predicate::str::contains("vector").not());
}

// ============================================================================
// .nodes — graph-specific paginated node browsing
// ============================================================================

#[test]
fn test_repl_nodes_graph_shows_header() {
    let (db_path, _temp) = setup_graph("kg");
    repl_run(&db_path, &[".nodes kg 1"])
        .stdout(predicate::str::contains("Nodes"))
        .stdout(predicate::str::contains("kg"));
}

#[test]
fn test_repl_nodes_graph_shows_node_ids() {
    let (db_path, _temp) = setup_graph("kg");
    // Edges: 10→11, 20→21, 30→31 — node IDs should appear
    repl_run(&db_path, &[".nodes kg"])
        .stdout(predicate::str::contains("10").or(predicate::str::contains("20")));
}

#[test]
fn test_repl_nodes_non_graph_returns_error() {
    let (db_path, _temp) = setup_vector("vecs", 4);
    repl_run(&db_path, &[".nodes vecs"])
        .stdout(predicate::str::contains("not found").or(predicate::str::contains("Error")));
}

// ============================================================================
// .export — Vector and Metadata; Graph returns informative error
// ============================================================================

#[test]
fn test_repl_export_vector_creates_file() {
    let (db_path, _temp) = setup_vector("vecs", 4);
    let export_path = db_path.parent().unwrap().join("vecs_export.json");

    repl_run(
        &db_path,
        &[&format!(".export vecs {}", export_path.display())],
    )
    .stdout(predicate::str::contains("Exported"));

    // File should exist and be valid JSON
    let content = std::fs::read_to_string(&export_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("should be valid JSON");
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 5);
}

#[test]
fn test_repl_export_metadata_no_vector_field() {
    let (db_path, _temp) = setup_metadata("meta");
    let export_path = db_path.parent().unwrap().join("meta_export.json");

    repl_run(
        &db_path,
        &[&format!(".export meta {}", export_path.display())],
    )
    .stdout(predicate::str::contains("Exported"));

    let content = std::fs::read_to_string(&export_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("should be valid JSON");
    let records = parsed.as_array().unwrap();
    assert_eq!(records.len(), 5);
    // No "vector" field in metadata export
    for record in records {
        assert!(
            record.get("vector").is_none(),
            "metadata export should not contain 'vector' field"
        );
        assert!(record.get("id").is_some());
    }
}

#[test]
fn test_repl_export_graph_returns_informative_error() {
    let (db_path, _temp) = setup_graph("kg");
    repl_run(&db_path, &[".export kg"])
        .stdout(predicate::str::contains("not supported").or(predicate::str::contains("Error")));
}

// ============================================================================
// .stats — shows collection-type-specific stats
// ============================================================================

#[test]
fn test_repl_stats_vector() {
    let (db_path, _temp) = setup_vector("vecs", 8);
    repl_run(&db_path, &[".stats vecs"])
        .stdout(predicate::str::contains("Vector"))
        .stdout(predicate::str::contains("Point Count"));
}

#[test]
fn test_repl_stats_graph() {
    let (db_path, _temp) = setup_graph("kg");
    repl_run(&db_path, &[".stats kg"])
        .stdout(predicate::str::contains("Graph"))
        .stdout(predicate::str::contains("Edge Count"));
}

#[test]
fn test_repl_stats_metadata() {
    let (db_path, _temp) = setup_metadata("meta");
    repl_run(&db_path, &[".stats meta"])
        .stdout(predicate::str::contains("Metadata"))
        .stdout(predicate::str::contains("Item Count"));
}

// ============================================================================
// VelesQL via REPL stdin
// ============================================================================

#[test]
fn test_repl_velesql_select_vector() {
    let (db_path, _temp) = setup_vector("vecs", 4);
    // REPL displays results in a table; "id" and "score" are always present columns
    repl_run(&db_path, &["SELECT * FROM vecs LIMIT 3"])
        .stdout(predicate::str::contains("id").and(predicate::str::contains("3 rows")));
}

#[test]
fn test_repl_velesql_select_metadata() {
    let (db_path, _temp) = setup_metadata("meta");
    repl_run(&db_path, &["SELECT * FROM meta LIMIT 5"])
        .stdout(predicate::str::contains("id").and(predicate::str::contains("rows")));
}

// ============================================================================
// .graph subcommands via REPL
// ============================================================================

#[test]
fn test_repl_graph_edges_command() {
    let (db_path, _temp) = setup_graph("kg");
    repl_run(&db_path, &[".graph edges kg"])
        .stdout(predicate::str::contains("LINKS").or(predicate::str::contains("edge")));
}

#[test]
fn test_repl_graph_degree_command() {
    let (db_path, _temp) = setup_graph("kg");
    // Node 10 has one outgoing edge (10→11)
    repl_run(&db_path, &[".graph degree kg 10"])
        .stdout(predicate::str::contains("Node Degree").or(predicate::str::contains("degree")));
}

// ============================================================================
// .schema and .describe
// ============================================================================

#[test]
fn test_repl_schema_vector() {
    let (db_path, _temp) = setup_vector("vecs", 4);
    repl_run(&db_path, &[".schema vecs"])
        .stdout(predicate::str::contains("Vector"))
        .stdout(predicate::str::contains("4").or(predicate::str::contains("dim")));
}

#[test]
fn test_repl_schema_graph() {
    let (db_path, _temp) = setup_graph("kg");
    repl_run(&db_path, &[".schema kg"]).stdout(predicate::str::contains("Graph"));
}

#[test]
fn test_repl_schema_metadata() {
    let (db_path, _temp) = setup_metadata("meta");
    repl_run(&db_path, &[".schema meta"]).stdout(predicate::str::contains("Metadata"));
}

// ============================================================================
// Multi-command session
// ============================================================================

#[test]
fn test_repl_multi_command_session() {
    let (db_path, _temp) = setup_vector("vecs", 4);

    // Send several commands in one session
    repl_run(
        &db_path,
        &[
            ".collections",
            ".count vecs",
            ".sample vecs 2",
            ".stats vecs",
        ],
    )
    .stdout(predicate::str::contains("vecs"))
    .stdout(predicate::str::contains("5"))
    .stdout(predicate::str::contains("Vector"));
}
