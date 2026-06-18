//! Unit tests for `repl_collection_cmds` — command dispatch, error/usage paths,
//! and the 3.0.0 display helpers (`index_health_label`, `print_metadata_detail`,
//! `print_node_page_body`, `node_entries_to_rows`, `vector_preview`).
//!
//! These exercise the private helpers directly (sibling-module access) plus the
//! `pub(crate)` `cmd_*` entry points by asserting on the returned
//! [`CommandResult`], which the binary-spawning e2e suite cannot reach for the
//! usage / not-found / empty-health branches.

use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, GraphEdge, GraphSchema, Point};

use super::{
    cmd_browse, cmd_count, cmd_describe, cmd_diagnostics, cmd_nodes, cmd_sample, cmd_schema,
    cmd_scroll, cmd_stats, index_health_label, node_entries_to_rows, print_node_page_body,
    vector_preview,
};
use crate::repl_commands::CommandResult;

// =============================================================================
// Fixtures
// =============================================================================

fn empty_db() -> (Database, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    (db, dir)
}

fn vector_db(name: &str, dim: usize, points: u64) -> (Database, TempDir) {
    let (db, dir) = empty_db();
    db.create_vector_collection(name, dim, DistanceMetric::Cosine)
        .unwrap();
    if points > 0 {
        let col = db.get_vector_collection(name).unwrap();
        for i in 1..=points {
            #[allow(clippy::cast_precision_loss)]
            let v: Vec<f32> = (0..dim).map(|j| (i as f32 + j as f32) / 10.0).collect();
            col.upsert(vec![Point {
                id: i,
                vector: v,
                payload: Some(serde_json::json!({"label": format!("v{i}")})),
                sparse_vectors: None,
            }])
            .unwrap();
        }
    }
    (db, dir)
}

fn graph_db(name: &str) -> (Database, TempDir) {
    let (db, dir) = empty_db();
    db.create_graph_collection(name, GraphSchema::schemaless())
        .unwrap();
    let col = db.get_graph_collection(name).unwrap();
    col.add_edge(GraphEdge::new(1, 10, 11, "LINKS").unwrap())
        .unwrap();
    col.upsert_node_payload(10, &serde_json::json!({"name": "n10"}))
        .unwrap();
    col.flush().unwrap();
    (db, dir)
}

fn metadata_db(name: &str, items: u64) -> (Database, TempDir) {
    let (db, dir) = empty_db();
    db.create_metadata_collection(name).unwrap();
    if items > 0 {
        let col = db.get_metadata_collection(name).unwrap();
        let pts: Vec<Point> = (1..=items)
            .map(|i| Point::metadata_only(i, serde_json::json!({"title": format!("t{i}")})))
            .collect();
        col.upsert(pts).unwrap();
    }
    (db, dir)
}

fn is_continue(r: &CommandResult) -> bool {
    matches!(r, CommandResult::Continue)
}

fn error_msg(r: &CommandResult) -> Option<&str> {
    match r {
        CommandResult::Error(m) => Some(m.as_str()),
        _ => None,
    }
}

// =============================================================================
// index_health_label — all variants (L378-382)
// =============================================================================

#[test]
fn test_index_health_label_healthy() {
    use velesdb_core::collection::IndexHealth;
    let (label, detail) = index_health_label(&IndexHealth::Healthy);
    assert_eq!(label, "healthy");
    assert!(detail.is_none());
}

#[test]
fn test_index_health_label_empty() {
    use velesdb_core::collection::IndexHealth;
    let (label, detail) = index_health_label(&IndexHealth::Empty);
    assert_eq!(label, "empty");
    assert!(detail.is_none());
}

#[test]
fn test_index_health_label_needs_rebuild_carries_reason() {
    use velesdb_core::collection::IndexHealth;
    let (label, detail) = index_health_label(&IndexHealth::NeedsRebuild("corrupt".into()));
    assert_eq!(label, "needs_rebuild");
    assert_eq!(detail.as_deref(), Some("corrupt"));
}

// =============================================================================
// print_node_page_body — non-empty branch
// =============================================================================

#[test]
fn test_print_node_page_body_non_empty_branch() {
    let entries = vec![
        (10u64, Some(serde_json::json!({"name": "a"}))),
        (20u64, None),
    ];
    // Renders the non-empty arm (table + "next page" hint) without panicking,
    // including a None payload. Assert on the rows the branch actually builds.
    let rows = node_entries_to_rows(&entries);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get("id"), Some(&serde_json::json!(10)));
    assert_eq!(rows[1].get("id"), Some(&serde_json::json!(20)));
    print_node_page_body(&entries, "kg", ".browse", 1);
}

// =============================================================================
// node_entries_to_rows (L444)
// =============================================================================

#[test]
fn test_node_entries_to_rows_maps_id_and_payload() {
    let entries = vec![(7u64, Some(serde_json::json!({"k": "v"}))), (8u64, None)];
    let rows = node_entries_to_rows(&entries);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], serde_json::json!(7));
    assert_eq!(rows[0]["k"], serde_json::json!("v"));
    assert_eq!(rows[0].len(), 2); // id + flattened payload key
    assert_eq!(rows[1]["id"], serde_json::json!(8));
    assert_eq!(rows[1].len(), 1); // None payload => only id, no spurious keys
}

#[test]
fn test_node_entries_to_rows_empty() {
    let rows = node_entries_to_rows(&[]);
    assert!(rows.is_empty());
}

// =============================================================================
// vector_preview (L488) — truncated vs. full
// =============================================================================

#[test]
fn test_vector_preview_truncates_long_vector() {
    let v: Vec<f32> = (0..10).map(|i| i as f32).collect();
    let preview = vector_preview(&v);
    assert!(preview.contains("... (10 dims)"), "got: {preview}");
}

#[test]
fn test_vector_preview_short_vector_no_ellipsis() {
    let preview = vector_preview(&[1.0, 2.0]);
    assert!(!preview.contains("..."), "got: {preview}");
    assert!(!preview.contains("dims"), "got: {preview}");
}

// =============================================================================
// cmd_diagnostics — usage (L342-344), not-found (L349), empty-health (L379)
// =============================================================================

#[test]
fn test_cmd_diagnostics_missing_arg_is_usage_continue() {
    let (db, _d) = empty_db();
    // Only the command word, no collection name -> prints Usage, returns Continue.
    assert!(is_continue(&cmd_diagnostics(&db, &[".diagnostics"])));
}

#[test]
fn test_cmd_diagnostics_unknown_collection_errors() {
    let (db, _d) = empty_db();
    let res = cmd_diagnostics(&db, &[".diagnostics", "ghost"]);
    assert!(error_msg(&res).unwrap().contains("ghost"));
}

#[test]
fn test_cmd_diagnostics_empty_collection_continue() {
    // 0 points -> diagnostics reports IndexHealth::Empty, exercising the "empty"
    // label arm of index_health_label inside cmd_diagnostics.
    let (db, _d) = vector_db("vecs", 4, 0);
    assert!(is_continue(&cmd_diagnostics(
        &db,
        &[".diagnostics", "vecs"]
    )));
}

#[test]
fn test_cmd_diagnostics_populated_collection_continue() {
    let (db, _d) = vector_db("vecs", 4, 3);
    assert!(is_continue(&cmd_diagnostics(
        &db,
        &[".diagnostics", "vecs"]
    )));
}

// =============================================================================
// cmd_describe — Metadata branch (L103 -> print_metadata_detail) + others
// =============================================================================

#[test]
fn test_cmd_describe_missing_arg_usage() {
    let (db, _d) = empty_db();
    assert!(is_continue(&cmd_describe(&db, &[".describe"])));
}

#[test]
fn test_cmd_describe_metadata_uses_shared_detail() {
    let (db, _d) = metadata_db("catalog", 4);
    assert!(is_continue(&cmd_describe(&db, &[".describe", "catalog"])));
}

#[test]
fn test_cmd_describe_vector_and_graph_continue() {
    let (db, _d) = vector_db("vecs", 4, 2);
    assert!(is_continue(&cmd_describe(&db, &[".describe", "vecs"])));
    let (gdb, _g) = graph_db("kg");
    assert!(is_continue(&cmd_describe(&gdb, &[".describe", "kg"])));
}

#[test]
fn test_cmd_describe_unknown_errors() {
    let (db, _d) = empty_db();
    assert!(error_msg(&cmd_describe(&db, &[".describe", "nope"]))
        .unwrap()
        .contains("nope"));
}

// =============================================================================
// cmd_stats — Metadata branch (L332 -> print_metadata_detail) + usage
// =============================================================================

#[test]
fn test_cmd_stats_missing_arg_usage() {
    let (db, _d) = empty_db();
    assert!(is_continue(&cmd_stats(&db, &[".stats"])));
}

#[test]
fn test_cmd_stats_metadata_branch() {
    let (db, _d) = metadata_db("catalog", 3);
    assert!(is_continue(&cmd_stats(&db, &[".stats", "catalog"])));
}

#[test]
fn test_cmd_stats_unknown_errors() {
    let (db, _d) = empty_db();
    assert!(error_msg(&cmd_stats(&db, &[".stats", "nope"])).is_some());
}

// =============================================================================
// cmd_schema / cmd_count / cmd_sample — usage + not-found branches
// =============================================================================

#[test]
fn test_cmd_schema_usage_metadata_and_unknown() {
    let (db, _d) = metadata_db("meta", 1);
    assert!(is_continue(&cmd_schema(&db, &[".schema"])));
    assert!(is_continue(&cmd_schema(&db, &[".schema", "meta"])));
    assert!(error_msg(&cmd_schema(&db, &[".schema", "ghost"])).is_some());
}

#[test]
fn test_cmd_count_usage_and_branches() {
    let (db, _d) = metadata_db("meta", 2);
    assert!(is_continue(&cmd_count(&db, &[".count"])));
    assert!(is_continue(&cmd_count(&db, &[".count", "meta"])));
    assert!(error_msg(&cmd_count(&db, &[".count", "ghost"])).is_some());
}

#[test]
fn test_cmd_sample_usage_graph_and_empty_rows() {
    let (db, _d) = graph_db("kg");
    assert!(is_continue(&cmd_sample(&db, &[".sample"])));
    // Graph branch produces node rows.
    assert!(is_continue(&cmd_sample(&db, &[".sample", "kg", "5"])));
    // Empty metadata collection -> print_sample_rows "No records found." branch.
    let (mdb, _m) = metadata_db("meta", 0);
    assert!(is_continue(&cmd_sample(&mdb, &[".sample", "meta"])));
    assert!(error_msg(&cmd_sample(&db, &[".sample", "ghost"])).is_some());
}

// =============================================================================
// cmd_browse — usage, graph out-of-range page (empty body), not-found
// =============================================================================

#[test]
fn test_cmd_browse_usage_and_graph_empty_page() {
    let (db, _d) = graph_db("kg");
    assert!(is_continue(&cmd_browse(&db, &[".browse"])));
    // Page far beyond the data -> graph node page is empty -> "No nodes on this page."
    assert!(is_continue(&cmd_browse(&db, &[".browse", "kg", "99"])));
}

#[test]
fn test_cmd_browse_metadata_empty_records() {
    // 0 items -> browse_id_based renders "No records on this page."
    let (db, _d) = metadata_db("meta", 0);
    assert!(is_continue(&cmd_browse(&db, &[".browse", "meta", "1"])));
}

#[test]
fn test_cmd_browse_unknown_errors() {
    let (db, _d) = empty_db();
    assert!(error_msg(&cmd_browse(&db, &[".browse", "ghost"])).is_some());
}

// =============================================================================
// cmd_nodes — usage, non-graph error, out-of-range empty page (L476 via .nodes)
// =============================================================================

#[test]
fn test_cmd_nodes_usage_and_non_graph_error() {
    let (db, _d) = vector_db("vecs", 4, 1);
    assert!(is_continue(&cmd_nodes(&db, &[".nodes"])));
    // Vector collection is not a graph -> error.
    assert!(error_msg(&cmd_nodes(&db, &[".nodes", "vecs"])).is_some());
}

#[test]
fn test_cmd_nodes_graph_out_of_range_empty_body() {
    let (db, _d) = graph_db("kg");
    // Valid graph but page beyond data -> empty-node-page body branch.
    assert!(is_continue(&cmd_nodes(&db, &[".nodes", "kg", "99"])));
}

// =============================================================================
// cmd_scroll — usage, invalid batch_size (0 & non-numeric), invalid cursor,
// not-found, and a successful scroll.
// =============================================================================

#[test]
fn test_cmd_scroll_usage_branch() {
    let (db, _d) = empty_db();
    assert!(is_continue(&cmd_scroll(&db, &[".scroll"])));
}

#[test]
fn test_cmd_scroll_zero_batch_size_errors() {
    let (db, _d) = vector_db("vecs", 4, 1);
    let res = cmd_scroll(&db, &[".scroll", "vecs", "0"]);
    assert!(error_msg(&res).unwrap().contains("batch_size"));
}

#[test]
fn test_cmd_scroll_non_numeric_batch_size_errors() {
    let (db, _d) = vector_db("vecs", 4, 1);
    let res = cmd_scroll(&db, &[".scroll", "vecs", "abc"]);
    assert!(error_msg(&res).unwrap().contains("batch_size"));
}

#[test]
fn test_cmd_scroll_invalid_cursor_errors() {
    let (db, _d) = vector_db("vecs", 4, 1);
    let res = cmd_scroll(&db, &[".scroll", "vecs", "10", "notanid"]);
    assert!(error_msg(&res).unwrap().contains("cursor"));
}

#[test]
fn test_cmd_scroll_unknown_collection_errors() {
    let (db, _d) = empty_db();
    assert!(error_msg(&cmd_scroll(&db, &[".scroll", "ghost"]))
        .unwrap()
        .contains("ghost"));
}

#[test]
fn test_cmd_scroll_success_continue() {
    let (db, _d) = vector_db("vecs", 4, 3);
    assert!(is_continue(&cmd_scroll(&db, &[".scroll", "vecs", "2"])));
}

#[test]
fn test_cmd_scroll_empty_collection_no_points() {
    // 0 points exercises print_scroll_results "No points found." branch.
    let (db, _d) = vector_db("vecs", 4, 0);
    assert!(is_continue(&cmd_scroll(&db, &[".scroll", "vecs"])));
}
