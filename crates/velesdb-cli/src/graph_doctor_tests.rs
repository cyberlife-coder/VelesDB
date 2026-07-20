//! BDD tests for `velesdb-cli graph doctor` (GIVEN -> WHEN -> THEN).
//!
//! Legacy phantom edges -- present in the edge store but with a source or
//! target that has no stored payload -- can only occur via WAL replay:
//! `Collection::open` never re-validates edges loaded from a pre-existing
//! WAL/snapshot (replay intentionally bypasses referential-integrity
//! checks so a legitimate edge-only database created before the #1442 fix
//! never silently loses data). See #1469 and `docs/CONCURRENCY_MODEL.md`.
//!
//! These tests simulate that scenario by writing a raw ADD entry directly
//! to `edges.wal`, bypassing `add_edge`'s referential-integrity validation,
//! then re-opening the collection so replay ingests it unchecked -- exactly
//! as a pre-#1442 database would look on disk.

use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use velesdb_core::collection::graph::GraphSchema;
use velesdb_core::{Database, GraphCollection, GraphEdge};

use crate::graph::GraphAction;

// =========================================================================
// Helpers
// =========================================================================

/// Create a fresh database + graph collection in a temp directory.
fn setup_graph_db() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db_path = dir.path().join("test_db");
    let db = Database::open(&db_path).expect("test: open database");
    db.create_graph_collection("kg", GraphSchema::schemaless())
        .expect("test: create graph collection");
    drop(db);
    (dir, db_path)
}

/// Open graph collection from path.
fn open_graph(path: &PathBuf) -> GraphCollection {
    let db = Database::open(path).expect("test: open database");
    db.get_graph_collection("kg")
        .expect("test: get graph collection")
}

/// WAL opcode for an Add entry. Mirrors `edge_wal.rs`'s private
/// `WAL_OP_ADD`; the on-disk layout is documented as a stable contract in
/// that module's doc comment (`[u32 body_len][u8 0x01][json(GraphEdge)]`).
const WAL_OP_ADD: u8 = 0x01;

/// Seeds a legacy phantom edge directly into `edges.wal`, bypassing
/// `add_edge`'s referential-integrity validation (#1442). Simulates a
/// pre-#1442 database on disk: the next `Collection::open` replays this
/// entry unchecked, exactly like a real legacy WAL/snapshot would.
fn seed_phantom_edge(db_path: &Path, id: u64, source: u64, target: u64, label: &str) {
    let edge = GraphEdge::new(id, source, target, label).expect("test: valid edge shape");
    let edge_bytes = serde_json::to_vec(&edge).expect("test: serialize edge");
    let body_len = u32::try_from(edge_bytes.len() + 1).expect("test: body fits u32");
    let wal_path = db_path.join("kg").join("edges.wal");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&wal_path)
        .expect("test: open edges.wal for append");
    f.write_all(&body_len.to_le_bytes())
        .expect("test: write len prefix");
    f.write_all(&[WAL_OP_ADD]).expect("test: write op byte");
    f.write_all(&edge_bytes).expect("test: write edge body");
    f.sync_all().expect("test: fsync wal");
}

/// Runs `graph doctor` with the given flags against the "kg" collection.
fn run_doctor(path: &Path, purge: bool, stub: bool) -> anyhow::Result<()> {
    crate::graph::handle(GraphAction::Doctor {
        path: path.to_path_buf(),
        collection: "kg".to_string(),
        purge,
        stub,
        format: "table".to_string(),
    })
}

// =========================================================================
// A. Nominal -- no phantoms
// =========================================================================

#[test]
fn test_doctor_no_phantoms_reports_clean_and_mutates_nothing() {
    // GIVEN: a graph with a single well-formed edge
    let (_dir, path) = setup_graph_db();
    let col = open_graph(&path);
    for id in [1, 2] {
        col.upsert_node_payload(id, &serde_json::json!({}))
            .expect("test: store payload");
    }
    col.add_edge(GraphEdge::new(1, 1, 2, "KNOWS").expect("test: valid edge"))
        .expect("test: add edge");
    col.flush().expect("test: flush");
    drop(col);

    // WHEN: doctor runs in default (report-only) mode
    run_doctor(&path, false, false).expect("doctor report should succeed");

    // THEN: nothing changed
    let col = open_graph(&path);
    assert_eq!(
        col.edge_count(),
        1,
        "a clean graph must be left untouched by doctor"
    );
}

// =========================================================================
// B. Phantom detected, dry-run default
// =========================================================================

#[test]
fn test_doctor_dry_run_detects_phantom_without_mutating() {
    // GIVEN: node 1 has a payload, node 2 never does, and a legacy WAL
    // entry links them with an edge that bypassed #1442 validation
    let (_dir, path) = setup_graph_db();
    let col = open_graph(&path);
    col.upsert_node_payload(1, &serde_json::json!({}))
        .expect("test: store payload");
    col.flush().expect("test: flush");
    drop(col);
    seed_phantom_edge(&path, 99, 1, 2, "LEGACY");

    // WHEN: doctor runs with no flags (dry-run report)
    run_doctor(&path, false, false).expect("dry-run report should succeed");

    // THEN: the phantom edge survives untouched and no stub was created
    let col = open_graph(&path);
    assert_eq!(
        col.edge_count(),
        1,
        "dry-run must never remove the phantom edge"
    );
    assert!(
        col.get_node_payload(2)
            .expect("test: get payload")
            .is_none(),
        "dry-run must never create a stub payload"
    );
}

// =========================================================================
// C. --purge
// =========================================================================

#[test]
fn test_doctor_purge_removes_phantom_edge_only() {
    // GIVEN: one valid edge (1->2) plus one legacy phantom edge (2->4,
    // node 4 has no payload)
    let (_dir, path) = setup_graph_db();
    let col = open_graph(&path);
    for id in [1, 2, 3] {
        col.upsert_node_payload(id, &serde_json::json!({}))
            .expect("test: store payload");
    }
    col.add_edge(GraphEdge::new(1, 1, 2, "VALID").expect("test: valid edge"))
        .expect("test: add edge");
    col.flush().expect("test: flush");
    drop(col);
    seed_phantom_edge(&path, 99, 2, 4, "LEGACY");

    // WHEN: doctor runs with --purge
    run_doctor(&path, true, false).expect("purge should succeed");

    // THEN: only the phantom edge is gone; the valid edge survives
    let col = open_graph(&path);
    let edges = col.get_edges(None);
    assert_eq!(edges.len(), 1, "only the phantom edge should be purged");
    assert!(
        edges.iter().any(|e| e.id() == 1),
        "doctor must never touch a valid edge"
    );
    assert!(
        !edges.iter().any(|e| e.id() == 99),
        "the phantom edge must be removed"
    );
}

#[test]
fn test_doctor_purge_is_idempotent() {
    // GIVEN: a single legacy phantom edge
    let (_dir, path) = setup_graph_db();
    let col = open_graph(&path);
    col.upsert_node_payload(1, &serde_json::json!({}))
        .expect("test: store payload");
    col.flush().expect("test: flush");
    drop(col);
    seed_phantom_edge(&path, 99, 1, 2, "LEGACY");

    // WHEN: purge runs twice
    run_doctor(&path, true, false).expect("first purge should succeed");
    let after_first = open_graph(&path).edge_count();
    run_doctor(&path, true, false).expect("second purge should be a no-op");
    let after_second = open_graph(&path).edge_count();

    // THEN: the second run makes no further changes
    assert_eq!(after_first, 0, "the phantom edge is removed on first purge");
    assert_eq!(
        after_second, 0,
        "running purge twice must produce no further changes"
    );
}

// =========================================================================
// D. --stub
// =========================================================================

#[test]
fn test_doctor_stub_seeds_empty_payload_for_missing_endpoint() {
    // GIVEN: a legacy phantom edge whose target (node 2) has no payload
    let (_dir, path) = setup_graph_db();
    let col = open_graph(&path);
    col.upsert_node_payload(1, &serde_json::json!({}))
        .expect("test: store payload");
    col.flush().expect("test: flush");
    drop(col);
    seed_phantom_edge(&path, 99, 1, 2, "LEGACY");

    // WHEN: doctor runs with --stub
    run_doctor(&path, false, true).expect("stub should succeed");

    // THEN: the edge is kept, and node 2 now has a minimal `{}` payload
    let col = open_graph(&path);
    assert_eq!(col.edge_count(), 1, "stub must keep the edge");
    assert_eq!(
        col.get_node_payload(2).expect("test: get payload"),
        Some(serde_json::json!({})),
        "the missing endpoint must be stubbed with an empty payload"
    );
    assert!(
        col.all_node_ids().contains(&2),
        "the stubbed node must now be visible to all_node_ids/MATCH"
    );
}

#[test]
fn test_doctor_stub_is_idempotent() {
    // GIVEN: a legacy phantom edge
    let (_dir, path) = setup_graph_db();
    let col = open_graph(&path);
    col.upsert_node_payload(1, &serde_json::json!({}))
        .expect("test: store payload");
    col.flush().expect("test: flush");
    drop(col);
    seed_phantom_edge(&path, 99, 1, 2, "LEGACY");

    // WHEN: stub runs twice
    run_doctor(&path, false, true).expect("first stub should succeed");
    run_doctor(&path, false, true).expect("second stub should be a no-op");

    // THEN: the node payload is still just the stub, edge still present
    let col = open_graph(&path);
    assert_eq!(col.edge_count(), 1);
    assert_eq!(
        col.get_node_payload(2).expect("test: get payload"),
        Some(serde_json::json!({})),
        "running stub twice must produce no further changes"
    );
}

// =========================================================================
// E. Doctor never touches a valid edge (regression)
// =========================================================================

#[test]
fn test_doctor_leaves_all_valid_edges_when_no_phantoms_exist_stub_mode() {
    // GIVEN: a graph with only valid edges
    let (_dir, path) = setup_graph_db();
    let col = open_graph(&path);
    for id in [1, 2, 3] {
        col.upsert_node_payload(id, &serde_json::json!({}))
            .expect("test: store payload");
    }
    col.add_edge(GraphEdge::new(1, 1, 2, "A").expect("test: valid edge"))
        .expect("test: add edge");
    col.add_edge(GraphEdge::new(2, 2, 3, "B").expect("test: valid edge"))
        .expect("test: add edge");
    col.flush().expect("test: flush");
    drop(col);

    // WHEN: --stub runs on a graph with no phantoms
    run_doctor(&path, false, true).expect("stub should succeed even with nothing to fix");

    // THEN: both edges and both payloads are untouched
    let col = open_graph(&path);
    assert_eq!(col.edge_count(), 2);
    assert_eq!(
        col.get_node_payload(1).expect("test: get payload"),
        Some(serde_json::json!({}))
    );
}
