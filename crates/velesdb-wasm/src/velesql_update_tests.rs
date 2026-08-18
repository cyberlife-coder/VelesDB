use super::*;
use crate::database::DatabaseInner;
use crate::velesql_value::parse_params;
use velesdb_core::velesql::{DmlStatement, Parser};

fn parse_update(sql: &str) -> UpdateStatement {
    let q = Parser::parse(sql).expect("test: parse");
    match q.dml.expect("test: has dml") {
        DmlStatement::Update(s) => s,
        other => panic!("expected UPDATE, got {other:?}"),
    }
}

fn seed_metadata_docs(db: &mut DatabaseInner) {
    db.create_metadata_collection("docs").expect("test: create");
    let store = db.get_shared_store("docs").expect("test: store");
    let mut borrowed = store.borrow_mut();
    borrowed.ids.push(1);
    borrowed
        .payloads
        .push(Some(serde_json::json!({"title": "first", "cat": "tech"})));
    borrowed.ids.push(2);
    borrowed
        .payloads
        .push(Some(serde_json::json!({"title": "second", "cat": "food"})));
}

#[test]
fn test_update_sets_field_on_match() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET title = 'renamed' WHERE id = 1");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update");
    assert_eq!(n, 1);

    let store = db.get_shared_store("docs").expect("test: store");
    let borrowed = store.borrow();
    let p = borrowed.payloads[0].as_ref().expect("test: payload");
    assert_eq!(p["title"], "renamed");
}

#[test]
fn test_update_without_where_affects_all_rows() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET cat = 'x'");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update all");
    assert_eq!(n, 2);
}

#[test]
fn test_update_on_id_column_is_rejected() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET id = 99 WHERE id = 1");
    let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("'id'"));
}

#[test]
fn test_update_on_vector_column_is_rejected() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let stmt = parse_update("UPDATE vecs SET vector = 'x' WHERE id = 1");
    let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("'vector'"));
}

#[test]
fn test_update_missing_collection_errors() {
    let db = DatabaseInner::new();
    let stmt = parse_update("UPDATE ghost SET x = 1 WHERE id = 1");
    let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
    assert!(err.is_err());
}

#[test]
fn test_update_no_rows_match_returns_zero() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET title = 'z' WHERE id = 999");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update");
    assert_eq!(n, 0);
}

#[test]
fn test_update_with_param() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET cat = $new WHERE id = 1");
    let params = parse_params(Some(r#"{"new": "gaming"}"#)).expect("test: p");
    let n = execute(&db, &stmt, &params).expect("test: update");
    assert_eq!(n, 1);

    let store = db.get_shared_store("docs").expect("test: store");
    let borrowed = store.borrow();
    let p = borrowed.payloads[0].as_ref().expect("test: payload");
    assert_eq!(p["cat"], "gaming");
}
