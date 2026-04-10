//! BDD integration tests for `VelesQL` admin statements: ANALYZE, TRUNCATE, ALTER COLLECTION.
//!
//! Tests the full pipeline: parse -> validate -> execute -> verify state.

use super::helpers::{create_test_db, execute_sql};
use velesdb_core::Point;

// ============================================================================
// ANALYZE — nominal
// ============================================================================

#[test]
fn test_analyze_returns_stats() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db.get_vector_collection("docs").expect("get collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], None),
    ])
    .expect("upsert");

    let results = execute_sql(&db, "ANALYZE docs").expect("ANALYZE should succeed");
    assert_eq!(results.len(), 1, "ANALYZE should return one result");

    let total = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("total_points"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(total, Some(3), "should report 3 total points");
}

#[test]
fn test_analyze_empty_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION empty (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(&db, "ANALYZE COLLECTION empty")
        .expect("ANALYZE empty collection should succeed");
    assert_eq!(results.len(), 1);

    let total = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("total_points"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(
        total,
        Some(0),
        "empty collection should have 0 total points"
    );
}

// ============================================================================
// TRUNCATE — nominal
// ============================================================================

#[test]
fn test_truncate_deletes_all_data() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION trunc (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db.get_vector_collection("trunc").expect("get collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], None),
        Point::new(4, vec![0.0, 0.0, 0.0, 1.0], None),
        Point::new(5, vec![1.0, 1.0, 0.0, 0.0], None),
    ])
    .expect("upsert");
    assert_eq!(vc.len(), 5);

    let results = execute_sql(&db, "TRUNCATE trunc").expect("TRUNCATE should succeed");
    assert_eq!(results.len(), 1, "TRUNCATE returns one result");

    let deleted = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("deleted_count"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(deleted, Some(5), "should report 5 deleted");

    assert_eq!(vc.len(), 0, "collection should be empty after TRUNCATE");
}

#[test]
fn test_truncate_empty_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION empty_trunc (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(&db, "TRUNCATE empty_trunc").expect("TRUNCATE empty should succeed");
    assert_eq!(results.len(), 1);

    let deleted = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("deleted_count"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(
        deleted,
        Some(0),
        "TRUNCATE on empty collection reports 0 deleted"
    );
}

#[test]
fn test_truncate_then_insert() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION reinsert (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db
        .get_vector_collection("reinsert")
        .expect("get collection");
    vc.upsert(vec![Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None)])
        .expect("upsert");

    execute_sql(&db, "TRUNCATE reinsert").expect("TRUNCATE");
    assert_eq!(vc.len(), 0, "should be empty after TRUNCATE");

    // Re-insert works normally
    vc.upsert(vec![Point::new(10, vec![0.0, 1.0, 0.0, 0.0], None)])
        .expect("re-insert");
    assert_eq!(vc.len(), 1, "re-insert should work after TRUNCATE");
}

// ============================================================================
// ALTER COLLECTION — honest rejection (Sprint 0 P0: stop the feature lie)
// ============================================================================

#[test]
fn test_alter_collection_set_option_is_honestly_rejected() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION alter_test (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    // ALTER COLLECTION SET used to return a misleading "status: accepted"
    // payload while silently discarding the requested value. The Sprint 0
    // fix replaces this feature lie with an honest rejection that
    // references the US-300 tracking ticket. A caller must now either
    // accept the error or use the drop+create workaround.
    let err = execute_sql(&db, "ALTER COLLECTION alter_test SET (auto_reindex = true)")
        .expect_err("ALTER COLLECTION SET must now return an error");

    let err_msg = err.to_string();
    assert!(
        err_msg.contains("US-300"),
        "error must reference US-300 tracking ticket, got: {err_msg}"
    );
    assert!(
        err_msg.contains("not yet implemented"),
        "error must state feature is not implemented, got: {err_msg}"
    );
}

// ============================================================================
// Complex / lifecycle scenarios
// ============================================================================

#[test]
fn test_analyze_truncate_lifecycle() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION lifecycle (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db
        .get_vector_collection("lifecycle")
        .expect("get collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], None),
    ])
    .expect("upsert");

    // ANALYZE with data
    let stats1 = execute_sql(&db, "ANALYZE lifecycle").expect("first ANALYZE");
    let count1 = stats1[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("total_points"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(count1, Some(3));

    // TRUNCATE
    execute_sql(&db, "TRUNCATE lifecycle").expect("TRUNCATE");
    assert_eq!(vc.len(), 0);

    // ANALYZE after TRUNCATE
    let stats2 = execute_sql(&db, "ANALYZE lifecycle").expect("second ANALYZE");
    let count2 = stats2[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("total_points"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(count2, Some(0));
}

#[test]
fn test_truncate_preserves_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION preserved (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db
        .get_vector_collection("preserved")
        .expect("get collection");
    vc.upsert(vec![Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None)])
        .expect("upsert");

    execute_sql(&db, "TRUNCATE COLLECTION preserved").expect("TRUNCATE");

    // Collection still shows in SHOW COLLECTIONS
    let show = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW");
    let names: Vec<&str> = show
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(serde_json::Value::as_str)
        })
        .collect();
    assert!(
        names.contains(&"preserved"),
        "collection should still exist after TRUNCATE"
    );

    // Can re-insert
    vc.upsert(vec![Point::new(99, vec![0.0, 0.0, 0.0, 1.0], None)])
        .expect("re-insert after TRUNCATE");
    assert_eq!(vc.len(), 1, "re-insert works after TRUNCATE");
}

// ============================================================================
// Negative — nonexistent collections
// ============================================================================

#[test]
fn test_analyze_nonexistent_fails() {
    let (_dir, db) = create_test_db();

    let result = execute_sql(&db, "ANALYZE ghost");
    assert!(result.is_err(), "ANALYZE nonexistent should fail");
    let msg = result.expect_err("error").to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "error should mention missing collection, got: {msg}"
    );
}

#[test]
fn test_truncate_nonexistent_fails() {
    let (_dir, db) = create_test_db();

    let result = execute_sql(&db, "TRUNCATE ghost");
    assert!(result.is_err(), "TRUNCATE nonexistent should fail");
    let msg = result.expect_err("error").to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "error should mention missing collection, got: {msg}"
    );
}

#[test]
fn test_alter_nonexistent_fails() {
    let (_dir, db) = create_test_db();

    let result = execute_sql(&db, "ALTER COLLECTION ghost SET (auto_reindex = true)");
    assert!(result.is_err(), "ALTER nonexistent should fail");
    let msg = result.expect_err("error").to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "error should mention missing collection, got: {msg}"
    );
}

// ============================================================================
// TRUNCATE on graph collections
// ============================================================================

#[test]
fn test_truncate_graph_collection_removes_nodes_and_edges() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create graph");

    // Insert nodes.
    let gc = db.get_graph_collection("kg").expect("get graph");
    gc.upsert_node_payload(1, &serde_json::json!({"name": "Alice"}))
        .expect("node 1");
    gc.upsert_node_payload(2, &serde_json::json!({"name": "Bob"}))
        .expect("node 2");

    // Insert edges.
    execute_sql(
        &db,
        "INSERT EDGE INTO kg (id = 10, source = 1, target = 2, label = 'KNOWS')",
    )
    .expect("edge");

    assert!(!gc.all_node_ids().is_empty(), "graph should have nodes");
    assert!(gc.edge_count() > 0, "graph should have edges");

    // TRUNCATE graph collection.
    let results = execute_sql(&db, "TRUNCATE kg").expect("TRUNCATE graph should succeed");
    assert_eq!(results.len(), 1);

    let payload = results[0].point.payload.as_ref().expect("payload");
    let deleted_count = payload
        .get("deleted_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(deleted_count > 0, "should have deleted nodes + edges");

    // Verify empty.
    let gc = db
        .get_graph_collection("kg")
        .expect("get graph after truncate");
    assert_eq!(gc.edge_count(), 0, "edges should be empty after TRUNCATE");
}

#[test]
fn test_truncate_empty_graph_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION empty_g (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create");

    // TRUNCATE on empty graph should succeed with 0 deleted.
    let results = execute_sql(&db, "TRUNCATE empty_g").expect("TRUNCATE empty graph");
    assert_eq!(results.len(), 1);

    let count = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("deleted_count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(999);
    assert_eq!(count, 0, "empty graph should report 0 deletions");
}

#[test]
fn test_truncate_graph_then_reinsert() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION reuse_g (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create");

    let gc = db.get_graph_collection("reuse_g").expect("get");
    gc.upsert_node_payload(1, &serde_json::json!({"name": "Alice"}))
        .expect("node");
    execute_sql(
        &db,
        "INSERT EDGE INTO reuse_g (id = 1, source = 1, target = 1, label = 'SELF')",
    )
    .expect("edge");

    // TRUNCATE then re-insert.
    execute_sql(&db, "TRUNCATE reuse_g").expect("truncate");

    let gc = db
        .get_graph_collection("reuse_g")
        .expect("get after truncate");
    assert_eq!(gc.edge_count(), 0);
    assert!(gc.all_node_ids().is_empty());

    // Re-insert should work normally.
    gc.upsert_node_payload(10, &serde_json::json!({"name": "Bob"}))
        .expect("re-insert node");
    execute_sql(
        &db,
        "INSERT EDGE INTO reuse_g (id = 20, source = 10, target = 10, label = 'NEW')",
    )
    .expect("re-insert edge");

    assert_eq!(gc.edge_count(), 1, "new edge should exist");
    assert!(!gc.all_node_ids().is_empty(), "new node should exist");
}

#[test]
fn test_truncate_graph_edges_only_no_node_payloads() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION edges_only (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create");

    // Insert edges without explicit node payloads (nodes exist as edge endpoints).
    execute_sql(
        &db,
        "INSERT EDGE INTO edges_only (id = 1, source = 100, target = 200, label = 'REF')",
    )
    .expect("edge");

    let gc = db.get_graph_collection("edges_only").expect("get");
    assert_eq!(gc.edge_count(), 1);

    // TRUNCATE should still succeed.
    let results = execute_sql(&db, "TRUNCATE edges_only").expect("truncate edges-only graph");
    assert_eq!(results.len(), 1);

    let gc = db.get_graph_collection("edges_only").expect("get after");
    assert_eq!(gc.edge_count(), 0, "edges should be cleared");
}
