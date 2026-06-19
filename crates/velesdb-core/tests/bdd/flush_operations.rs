//! BDD integration tests for `VelesQL` FLUSH statement (`VelesQL` v3.6).
//!
//! Tests the full pipeline: parse -> validate -> execute -> verify.
//! FLUSH persists collection data to disk; these tests verify the
//! operation completes without error and returns the expected payload.

use super::helpers::{create_test_db, execute_sql};
use velesdb_core::Point;

// ============================================================================
// FLUSH — all collections (fast)
// ============================================================================

#[test]
fn test_flush_all_succeeds() {
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
    ])
    .expect("upsert");

    let results = execute_sql(&db, "FLUSH").expect("FLUSH should succeed");
    assert_eq!(results.len(), 1, "FLUSH returns one status result");

    let status = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("status"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(status, Some("flushed"));

    let full = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("full"))
        .and_then(serde_json::Value::as_bool);
    assert_eq!(full, Some(false), "bare FLUSH should not be full");
}

// ============================================================================
// FLUSH FULL — all collections (full flush)
// ============================================================================

#[test]
fn test_flush_full_succeeds() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db.get_vector_collection("docs").expect("get collection");
    vc.upsert(vec![Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None)])
        .expect("upsert");

    let results = execute_sql(&db, "FLUSH FULL").expect("FLUSH FULL should succeed");
    assert_eq!(results.len(), 1);

    let full = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("full"))
        .and_then(serde_json::Value::as_bool);
    assert_eq!(full, Some(true), "FLUSH FULL should report full=true");
}

// ============================================================================
// FLUSH <collection> — specific collection
// ============================================================================

#[test]
fn test_flush_specific_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db.get_vector_collection("docs").expect("get collection");
    vc.upsert(vec![Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None)])
        .expect("upsert");

    let results = execute_sql(&db, "FLUSH docs").expect("FLUSH docs should succeed");
    assert_eq!(results.len(), 1);

    let status = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("status"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(status, Some("flushed"));
}

// ============================================================================
// Negative — nonexistent collection
// ============================================================================

#[test]
fn test_flush_nonexistent_collection_fails() {
    let (_dir, db) = create_test_db();

    let result = execute_sql(&db, "FLUSH ghost");
    assert!(result.is_err(), "FLUSH nonexistent should fail");
    let msg = result.expect_err("error").to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "error should mention missing collection, got: {msg}"
    );
}

// ============================================================================
// FLUSH FULL <collection> — specific collection, full mode
// ============================================================================

#[test]
fn test_flush_full_specific_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION archive (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db.get_vector_collection("archive").expect("get collection");
    vc.upsert(vec![Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None)])
        .expect("upsert");

    let results =
        execute_sql(&db, "FLUSH FULL archive").expect("FLUSH FULL archive should succeed");
    assert_eq!(results.len(), 1);

    let full = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("full"))
        .and_then(serde_json::Value::as_bool);
    assert_eq!(full, Some(true));
}

// ============================================================================
// FLUSH on empty database — no-op, no error
// ============================================================================

#[test]
fn test_flush_empty_database() {
    let (_dir, db) = create_test_db();

    let results = execute_sql(&db, "FLUSH").expect("FLUSH on empty DB should succeed");
    assert_eq!(
        results.len(),
        1,
        "FLUSH returns one status result even with no collections"
    );

    let status = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("status"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(status, Some("flushed"));

    let full = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("full"))
        .and_then(serde_json::Value::as_bool);
    assert_eq!(
        full,
        Some(false),
        "bare FLUSH on empty DB should not be full"
    );
}
