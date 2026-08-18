use super::*;
use crate::database::DatabaseInner;

fn new_db_with_metadata() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    db
}

#[test]
fn test_execute_create_collection() {
    let mut db = DatabaseInner::new();
    let r = execute(
        &mut db,
        "CREATE COLLECTION vecs (dimension = 4, metric = 'cosine')",
        None,
    )
    .expect("test: ddl");
    assert_eq!(r.kind(), "ddl");
    assert!(db.contains("vecs"));
}

#[test]
fn test_execute_invalid_sql_returns_error() {
    let mut db = DatabaseInner::new();
    let err = execute(&mut db, "NOT VALID AT ALL", None);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("parse error"));
}

#[test]
fn test_execute_rejects_train() {
    let mut db = new_db_with_metadata();
    let err = execute(&mut db, "TRAIN QUANTIZER ON docs WITH (type = 'sq8')", None);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("TRAIN"));
}

#[test]
fn test_execute_accepts_union() {
    let mut db = new_db_with_metadata();
    // Empty union should return 0 rows rather than erroring.
    let r =
        execute(&mut db, "SELECT * FROM docs UNION SELECT * FROM docs", None).expect("test: union");
    assert_eq!(r.row_count(), 0);
}

// --- Finding F13: DML result exposes row_count without placeholders ---
//
// Pre-F13: INSERT of N rows built N placeholder QueryResultRow objects
// (`{"id":0,"score":0.0}`) just so rows.len() matched the count. We
// now store the count explicitly and leave rows empty.

#[test]
fn test_dml_result_exposes_row_count_without_placeholder_rows() {
    let mut db = new_db_with_metadata();
    let r = execute(
        &mut db,
        "INSERT INTO docs (id, title) VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd'), (5, 'e')",
        None,
    )
    .expect("test: insert");
    assert_eq!(r.kind(), "mutation");
    assert_eq!(r.row_count(), 5, "row_count reports affected count");
    // rowsJson is the empty array for DML — no placeholder rows.
    assert_eq!(r.rows_json(), "[]");
    // Internal accessor: the row vec is empty.
    assert_eq!(r.rows_ref().len(), 0);
}

#[test]
fn test_update_dml_row_count_matches_affected() {
    let mut db = new_db_with_metadata();
    execute(
        &mut db,
        "INSERT INTO docs (id, cat) VALUES (1, 'a'), (2, 'a'), (3, 'b')",
        None,
    )
    .expect("test: seed");
    let r =
        execute(&mut db, "UPDATE docs SET cat = 'z' WHERE cat = 'a'", None).expect("test: update");
    assert_eq!(r.row_count(), 2);
    assert_eq!(r.rows_json(), "[]");
}

#[test]
fn test_delete_dml_row_count_matches_affected() {
    let mut db = new_db_with_metadata();
    execute(
        &mut db,
        "INSERT INTO docs (id, n) VALUES (1, 1), (2, 2), (3, 3)",
        None,
    )
    .expect("test: seed");
    let r = execute(&mut db, "DELETE FROM docs WHERE n > 1", None).expect("test: delete");
    assert_eq!(r.kind(), "deletion");
    assert_eq!(r.row_count(), 2);
    assert_eq!(r.rows_json(), "[]");
}

#[test]
fn test_select_still_materialises_rows() {
    // Non-regression: row-returning statements keep their rows — the
    // mutation_count optimization must not affect SELECT paths.
    let mut db = new_db_with_metadata();
    execute(
        &mut db,
        "INSERT INTO docs (id, n) VALUES (1, 10), (2, 20)",
        None,
    )
    .expect("test: seed");
    let r = execute(&mut db, "SELECT * FROM docs", None).expect("test: select");
    assert_eq!(r.kind(), "rows");
    assert_eq!(r.row_count(), 2);
    // rowsJson is a real JSON array with two non-placeholder objects.
    let rj = r.rows_json();
    assert!(rj.starts_with('['));
    assert!(rj.contains("\"n\":10") || rj.contains("\"n\":20"));
}
