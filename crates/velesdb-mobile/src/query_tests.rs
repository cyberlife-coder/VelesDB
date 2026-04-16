//! Integration tests for `VelesDatabase::execute_query()` (S4-14).

use crate::query::QueryResultKind;
use crate::types::{DistanceMetric, VelesError};
use crate::VelesDatabase;
use tempfile::TempDir;

/// Opens a temp database with a metadata-only collection named `docs`.
///
/// Metadata collections accept INSERT without a `vector` column, which
/// makes them ideal for testing the full VelesQL CRUD surface.
fn setup_db_with_metadata() -> (TempDir, std::sync::Arc<VelesDatabase>) {
    let tmp = TempDir::new().expect("test: create temp dir");
    let path = tmp.path().to_str().expect("test: path to str").to_string();
    let db = VelesDatabase::open(path).expect("test: open database");
    db.create_metadata_collection("docs".to_string())
        .expect("test: create metadata collection");
    (tmp, db)
}

/// Opens a temp database with a 4-dim cosine vector collection named `vecs`.
fn setup_db_with_vector_collection() -> (TempDir, std::sync::Arc<VelesDatabase>) {
    let tmp = TempDir::new().expect("test: create temp dir");
    let path = tmp.path().to_str().expect("test: path to str").to_string();
    let db = VelesDatabase::open(path).expect("test: open database");
    db.create_collection("vecs".to_string(), 4, DistanceMetric::Cosine)
        .expect("test: create vector collection");
    (tmp, db)
}

/// Inserts seed data into metadata collection `docs` via VelesQL INSERT.
fn seed_metadata_docs(db: &VelesDatabase) {
    let sql = concat!(
        "INSERT INTO docs (id, title, category) VALUES ",
        "(1, 'Rust Programming', 'tech'), ",
        "(2, 'Cooking Basics', 'food'), ",
        "(3, 'Advanced Algorithms', 'tech')"
    );
    let result = db
        .execute_query(sql.to_string(), None)
        .expect("test: seed INSERT into metadata collection");
    assert!(
        matches!(result.kind, QueryResultKind::Mutation),
        "INSERT should return Mutation kind"
    );
    assert_eq!(result.row_count, 3, "3 rows should be inserted");
}

// =========================================================================
// SELECT tests
// =========================================================================

#[test]
fn test_execute_query_select_returns_rows() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let result = db
        .execute_query("SELECT * FROM docs LIMIT 10".to_string(), None)
        .expect("test: SELECT should succeed");

    assert!(matches!(result.kind, QueryResultKind::Rows));
    assert!(result.row_count >= 3, "should return at least 3 rows");
    assert!(result.message.contains("row(s) returned"));
}

#[test]
fn test_execute_query_select_row_contains_payload() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let result = db
        .execute_query("SELECT * FROM docs LIMIT 10".to_string(), None)
        .expect("test: SELECT with payload");

    let has_title = result
        .rows
        .iter()
        .any(|row| row.data_json.contains("title"));
    assert!(has_title, "rows should contain title payload field");
}

// =========================================================================
// INSERT tests
// =========================================================================

#[test]
fn test_execute_query_insert_adds_data() {
    let (_tmp, db) = setup_db_with_metadata();

    let result = db
        .execute_query(
            "INSERT INTO docs (id, title) VALUES (10, 'New Doc')".to_string(),
            None,
        )
        .expect("test: INSERT");

    assert!(matches!(result.kind, QueryResultKind::Mutation));
    assert_eq!(result.row_count, 1);

    // Verify the data is retrievable via SELECT
    let select = db
        .execute_query("SELECT * FROM docs LIMIT 10".to_string(), None)
        .expect("test: SELECT after INSERT");
    assert!(
        select.rows.iter().any(|r| r.id == 10),
        "inserted point should be retrievable"
    );
}

#[test]
fn test_execute_query_multi_row_insert() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let count = db
        .execute_query("SELECT * FROM docs LIMIT 100".to_string(), None)
        .expect("test: count rows")
        .row_count;
    assert_eq!(count, 3);
}

// =========================================================================
// INSERT with vector (vector collection, requires $param for vectors)
// =========================================================================

#[test]
fn test_execute_query_insert_with_vector_param() {
    let (_tmp, db) = setup_db_with_vector_collection();

    // VelesQL requires vector data via $parameter substitution
    let sql = "INSERT INTO vecs (id, vector, tag) VALUES (1, $v, 'a')";
    let params = r#"{"v": [1.0, 0.0, 0.0, 0.0]}"#;
    let result = db
        .execute_query(sql.to_string(), Some(params.to_string()))
        .expect("test: INSERT with vector param");

    assert!(matches!(result.kind, QueryResultKind::Mutation));
    assert_eq!(result.row_count, 1);
}

// =========================================================================
// UPDATE tests
// =========================================================================

#[test]
fn test_execute_query_update_modifies_data() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let result = db
        .execute_query(
            "UPDATE docs SET title = 'Updated' WHERE id = 1".to_string(),
            None,
        )
        .expect("test: UPDATE");

    assert!(matches!(result.kind, QueryResultKind::Mutation));
    assert!(result.row_count >= 1, "at least 1 row should be updated");

    // Verify update took effect
    let select = db
        .execute_query("SELECT * FROM docs LIMIT 10".to_string(), None)
        .expect("test: SELECT after UPDATE");
    let updated_row = select.rows.iter().find(|r| r.id == 1);
    assert!(updated_row.is_some(), "point 1 should still exist");
    assert!(
        updated_row
            .expect("test: row exists")
            .data_json
            .contains("Updated"),
        "title should be updated"
    );
}

// =========================================================================
// DELETE tests
// =========================================================================

#[test]
fn test_execute_query_delete_removes_data() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let result = db
        .execute_query("DELETE FROM docs WHERE id = 2".to_string(), None)
        .expect("test: DELETE");

    assert!(matches!(result.kind, QueryResultKind::Deletion));

    // Verify deletion
    let select = db
        .execute_query("SELECT * FROM docs LIMIT 10".to_string(), None)
        .expect("test: SELECT after DELETE");
    assert!(
        !select.rows.iter().any(|r| r.id == 2),
        "point 2 should be deleted"
    );
}

// =========================================================================
// DDL tests
// =========================================================================

#[test]
fn test_execute_query_create_collection() {
    let tmp = TempDir::new().expect("test: temp dir");
    let path = tmp.path().to_str().expect("test: path").to_string();
    let db = VelesDatabase::open(path).expect("test: open db");

    let result = db
        .execute_query(
            "CREATE COLLECTION new_coll (dimension = 128, metric = 'cosine')".to_string(),
            None,
        )
        .expect("test: CREATE COLLECTION");

    assert!(matches!(result.kind, QueryResultKind::Ddl));
    assert!(result.message.contains("DDL"));

    // Verify the collection exists
    assert!(db.list_collections().contains(&"new_coll".to_string()));
}

#[test]
fn test_execute_query_drop_collection() {
    let (_tmp, db) = setup_db_with_metadata();

    let result = db
        .execute_query("DROP COLLECTION docs".to_string(), None)
        .expect("test: DROP COLLECTION");

    assert!(matches!(result.kind, QueryResultKind::Ddl));
    assert!(!db.list_collections().contains(&"docs".to_string()));
}

// =========================================================================
// Introspection tests
// =========================================================================

#[test]
fn test_execute_query_show_collections() {
    let (_tmp, db) = setup_db_with_metadata();

    let result = db
        .execute_query("SHOW COLLECTIONS".to_string(), None)
        .expect("test: SHOW COLLECTIONS");

    assert!(matches!(result.kind, QueryResultKind::Rows));
    assert!(
        result.row_count >= 1,
        "SHOW COLLECTIONS should list at least 1 collection"
    );
}

// =========================================================================
// Admin tests
// =========================================================================

#[test]
fn test_execute_query_flush() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let result = db
        .execute_query("FLUSH FULL".to_string(), None)
        .expect("test: FLUSH");

    assert!(matches!(result.kind, QueryResultKind::Admin));
    assert!(result.message.contains("Admin"));
}

// =========================================================================
// Error handling tests (negative cases)
// =========================================================================

#[test]
fn test_execute_query_invalid_sql_returns_error() {
    let (_tmp, db) = setup_db_with_metadata();

    let result = db.execute_query("NOT VALID SQL AT ALL".to_string(), None);

    assert!(result.is_err(), "invalid SQL should return an error");
    let err = result.expect_err("test: error expected");
    match err {
        VelesError::Database { message } => {
            assert!(
                message.contains("parse error"),
                "error should mention parse error, got: {message}"
            );
        }
        other => panic!("expected VelesError::Database, got: {other:?}"),
    }
}

#[test]
fn test_execute_query_nonexistent_collection() {
    let tmp = TempDir::new().expect("test: temp dir");
    let path = tmp.path().to_str().expect("test: path").to_string();
    let db = VelesDatabase::open(path).expect("test: open db");

    let result = db.execute_query("SELECT * FROM ghost_collection LIMIT 5".to_string(), None);

    assert!(
        result.is_err(),
        "query on nonexistent collection should fail"
    );
}

#[test]
fn test_execute_query_invalid_params_json() {
    let (_tmp, db) = setup_db_with_metadata();

    let result = db.execute_query(
        "SELECT * FROM docs LIMIT 5".to_string(),
        Some("not-json".to_string()),
    );

    assert!(result.is_err(), "invalid params JSON should fail");
}

#[test]
fn test_execute_query_vector_insert_missing_vector() {
    let (_tmp, db) = setup_db_with_vector_collection();

    // INSERT without vector column on a vector collection should fail
    let result = db.execute_query(
        "INSERT INTO vecs (id, title) VALUES (1, 'no vec')".to_string(),
        None,
    );

    assert!(
        result.is_err(),
        "INSERT without vector on vector collection should fail"
    );
}

// =========================================================================
// train_pq non-regression
// =========================================================================

#[test]
fn test_train_pq_still_works_after_execute_query() {
    let (_tmp, db) = setup_db_with_vector_collection();

    // Insert a point via $param so the collection has data
    let sql = "INSERT INTO vecs (id, vector) VALUES (1, $v)";
    let params = r#"{"v": [1.0, 0.0, 0.0, 0.0]}"#;
    let _ = db.execute_query(sql.to_string(), Some(params.to_string()));

    let config = crate::types::PqTrainConfig {
        m: 2,
        k: 4,
        opq: false,
    };
    // PQ training on 1 point will likely fail (insufficient data),
    // but the important thing is it reaches the training path without panic.
    let _result = db.train_pq("vecs".to_string(), config);
}

// =========================================================================
// Params forwarding test
// =========================================================================

#[test]
fn test_execute_query_with_params() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let result = db
        .execute_query(
            "SELECT * FROM docs LIMIT 5".to_string(),
            Some("{}".to_string()),
        )
        .expect("test: query with empty params");

    assert!(matches!(result.kind, QueryResultKind::Rows));
    assert!(result.row_count >= 1);
}

// =========================================================================
// QueryResult structure tests
// =========================================================================

#[test]
fn test_query_result_row_json_structure() {
    let (_tmp, db) = setup_db_with_metadata();
    seed_metadata_docs(&db);

    let result = db
        .execute_query("SELECT * FROM docs LIMIT 1".to_string(), None)
        .expect("test: SELECT for row structure");

    assert!(!result.rows.is_empty(), "should return at least 1 row");
    let row = &result.rows[0];

    // Verify the JSON is parseable
    let parsed: serde_json::Value =
        serde_json::from_str(&row.data_json).expect("test: row data_json should be valid JSON");
    assert!(
        parsed.get("id").is_some(),
        "row JSON should contain 'id' field"
    );
    assert!(
        parsed.get("score").is_some(),
        "row JSON should contain 'score' field"
    );
}
