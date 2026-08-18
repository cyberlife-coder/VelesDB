use super::*;
use crate::database::DatabaseInner;
use crate::velesql_value::parse_params;
use velesdb_core::velesql::{DmlStatement, Parser};

fn parse_insert(sql: &str) -> InsertStatement {
    let q = Parser::parse(sql).expect("test: parse");
    match q.dml.expect("test: has dml") {
        DmlStatement::Insert(s) | DmlStatement::Upsert(s) => s,
        other => panic!("expected INSERT/UPSERT, got {other:?}"),
    }
}

fn empty_params() -> Params {
    parse_params(None).expect("test: empty params")
}

#[test]
fn test_insert_single_metadata_row() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    let stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'hello')");
    let n = execute(&db, &stmt, &empty_params()).expect("test: insert");
    assert_eq!(n, 1);
}

#[test]
fn test_insert_multi_row_metadata() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    let stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'a'), (2, 'b'), (3, 'c')");
    let n = execute(&db, &stmt, &empty_params()).expect("test: insert");
    assert_eq!(n, 3);
}

#[test]
fn test_insert_missing_id_column_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    let stmt = parse_insert("INSERT INTO docs (title) VALUES ('hello')");
    let err = execute(&db, &stmt, &empty_params());
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("'id'"));
}

#[test]
fn test_insert_vector_collection_without_vector_errors() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let stmt = parse_insert("INSERT INTO vecs (id, title) VALUES (1, 'x')");
    let err = execute(&db, &stmt, &empty_params());
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("vector"));
}

#[test]
fn test_insert_vector_collection_with_param() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let stmt = parse_insert("INSERT INTO vecs (id, vector, tag) VALUES (1, $v, 'a')");
    let params = parse_params(Some(r#"{"v": [1.0, 0.0, 0.0, 0.0]}"#)).expect("test: parse params");
    let n = execute(&db, &stmt, &params).expect("test: insert");
    assert_eq!(n, 1);
}

#[test]
fn test_insert_vector_dimension_mismatch_errors() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let stmt = parse_insert("INSERT INTO vecs (id, vector) VALUES (1, $v)");
    let params = parse_params(Some(r#"{"v": [1.0, 0.0]}"#)).expect("test: parse params");
    let err = execute(&db, &stmt, &params);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("dimension mismatch"));
}

#[test]
fn test_insert_missing_collection_errors() {
    let db = DatabaseInner::new();
    let stmt = parse_insert("INSERT INTO ghost (id, t) VALUES (1, 'x')");
    let err = execute(&db, &stmt, &empty_params());
    assert!(err.is_err());
    assert!(err
        .expect_err("test: err")
        .contains("Collection 'ghost' not found"));
}

#[test]
fn test_upsert_replaces_existing_row() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");

    // First insert
    let stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'first')");
    execute(&db, &stmt, &empty_params()).expect("test: first insert");

    // Second insert with same id — should replace
    let stmt2 = parse_insert("UPSERT INTO docs (id, title) VALUES (1, 'second')");
    execute(&db, &stmt2, &empty_params()).expect("test: upsert");

    // Verify only one row remains
    let store = db.get_shared_store("docs").expect("test: store");
    assert_eq!(store.borrow().len(), 1);
}

#[test]
fn test_insert_rejects_row_arity_mismatch_at_parse_time() {
    // The parser rejects row-arity mismatches, so this is mostly a
    // defensive sanity check for the validate_statement path when a row
    // is manually crafted.
    let mut stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'a')");
    stmt.rows.push(vec![Value::Integer(2)]); // arity mismatch
    let db = DatabaseInner::new();
    let err = execute(&db, &stmt, &empty_params());
    assert!(err.is_err());
}
