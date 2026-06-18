//! E2E lifecycle tests for `VelesQL` `CREATE INDEX` / `DROP INDEX`.
//!
//! Each test exercises the full pipeline: SQL string -> parse -> execute ->
//! verify database state. Isolated via `tempfile::TempDir`, requires
//! `persistence` feature.

use super::helpers::{create_test_db, execute_sql};
use velesdb_core::Point;

// =========================================================================
// Scenario 1: CREATE INDEX on a field
// =========================================================================

#[test]
fn test_create_index_on_field() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE COLLECTION");

    let result = execute_sql(&db, "CREATE INDEX ON docs (category);");
    assert!(result.is_ok(), "CREATE INDEX should succeed");
    assert!(
        result.expect("ok").is_empty(),
        "DDL returns empty result set"
    );

    let indexed = db.indexed_fields_for("docs");
    assert!(
        indexed.contains("category"),
        "CREATE INDEX must register the field as a secondary index"
    );
}

// =========================================================================
// Scenario 2: CREATE INDEX improves lookup (verifies index exists)
// =========================================================================

#[test]
fn test_create_index_improves_lookup() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION indexed_coll (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE COLLECTION");

    let vc = db
        .get_vector_collection("indexed_coll")
        .expect("test: get collection");

    // Insert data with payload
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"category": "tech"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"category": "science"})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"category": "tech"})),
        ),
    ])
    .expect("test: upsert");

    // Create index via SQL
    execute_sql(&db, "CREATE INDEX ON indexed_coll (category);")
        .expect("test: CREATE INDEX should succeed");

    // Query with WHERE on indexed field
    let results = execute_sql(
        &db,
        "SELECT * FROM indexed_coll WHERE category = 'tech' LIMIT 10;",
    )
    .expect("test: SELECT with filter");

    assert_eq!(results.len(), 2, "Should find 2 'tech' documents");
}

// =========================================================================
// Scenario 3: DROP INDEX removes the index
// =========================================================================

#[test]
fn test_drop_index_removes_index() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION drop_idx (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE COLLECTION");

    execute_sql(&db, "CREATE INDEX ON drop_idx (category);").expect("test: CREATE INDEX");

    let vc = db
        .get_vector_collection("drop_idx")
        .expect("test: get collection");
    assert!(
        vc.has_secondary_index("category"),
        "index should exist before DROP",
    );

    let result = execute_sql(&db, "DROP INDEX ON drop_idx (category);");
    assert!(result.is_ok(), "DROP INDEX should succeed");
    assert!(
        result.expect("ok").is_empty(),
        "DDL returns empty result set"
    );
    assert!(
        !vc.has_secondary_index("category"),
        "DROP INDEX must remove the secondary index entry",
    );
}

// =========================================================================
// Scenario 4: CREATE INDEX on nonexistent collection fails
// =========================================================================

#[test]
fn test_create_index_on_nonexistent_collection_fails() {
    let (_dir, db) = create_test_db();

    let err = execute_sql(&db, "CREATE INDEX ON ghost (field);")
        .expect_err("CREATE INDEX on nonexistent collection should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "Error should mention missing collection, got: {msg}"
    );
}

// =========================================================================
// Scenario 5: DROP INDEX on nonexistent collection fails
// =========================================================================

#[test]
fn test_drop_index_on_nonexistent_collection_fails() {
    let (_dir, db) = create_test_db();

    let err = execute_sql(&db, "DROP INDEX ON ghost (field);")
        .expect_err("DROP INDEX on nonexistent collection should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "Error should mention missing collection, got: {msg}"
    );
}

// =========================================================================
// Scenario 6: Full index lifecycle
// =========================================================================

#[test]
fn test_create_index_lifecycle() {
    let (_dir, db) = create_test_db();

    // Step 1: Create collection
    execute_sql(
        &db,
        "CREATE COLLECTION lifecycle (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE COLLECTION");

    // Step 2: Insert data
    let vc = db
        .get_vector_collection("lifecycle")
        .expect("test: get collection");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"status": "active"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"status": "inactive"})),
        ),
    ])
    .expect("test: upsert");

    // Step 3: Create index
    execute_sql(&db, "CREATE INDEX ON lifecycle (status);").expect("test: CREATE INDEX");

    // Step 4: Query with indexed field
    let results = execute_sql(
        &db,
        "SELECT * FROM lifecycle WHERE status = 'active' LIMIT 10;",
    )
    .expect("test: SELECT with indexed field");
    assert_eq!(results.len(), 1, "Should find 1 active document");

    // Step 5: Drop index
    execute_sql(&db, "DROP INDEX ON lifecycle (status);").expect("test: DROP INDEX");

    // Step 6: Query still works without index (falls back to scan)
    let results = execute_sql(
        &db,
        "SELECT * FROM lifecycle WHERE status = 'active' LIMIT 10;",
    )
    .expect("test: SELECT after DROP INDEX should still work");
    assert_eq!(
        results.len(),
        1,
        "Should still find 1 active document after dropping index"
    );
}

// =========================================================================
// Scenario 7: Duplicate index is idempotent
// =========================================================================

#[test]
fn test_create_duplicate_index_is_idempotent() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION idempotent (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE COLLECTION");

    // Create same index twice -- should not error
    execute_sql(&db, "CREATE INDEX ON idempotent (category);").expect("test: first CREATE INDEX");
    execute_sql(&db, "CREATE INDEX ON idempotent (category);")
        .expect("test: second CREATE INDEX should be idempotent");
}

// =========================================================================
// Scenario 8: DROP INDEX on non-existent field is silent
// =========================================================================

#[test]
fn test_drop_index_nonexistent_field_succeeds() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION silent_drop (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE COLLECTION");

    // Drop index on a field that was never indexed -- should succeed silently
    let result = execute_sql(&db, "DROP INDEX ON silent_drop (nonexistent_field);");
    assert!(
        result.is_ok(),
        "DROP INDEX on non-indexed field should succeed silently"
    );
}
