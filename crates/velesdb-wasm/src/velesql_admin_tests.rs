use super::*;
use crate::database::DatabaseInner;
use velesdb_core::velesql::Parser;

fn parse_admin(sql: &str) -> AdminStatement {
    let q = Parser::parse(sql).expect("test: parse");
    q.admin.expect("test: has admin")
}

#[test]
fn test_flush_no_collection_is_noop() {
    let db = DatabaseInner::new();
    let msg = execute(&db, &parse_admin("FLUSH")).expect("test: flush");
    assert!(msg.contains("no-op"));
}

#[test]
fn test_flush_full_no_collection_is_noop() {
    let db = DatabaseInner::new();
    let msg = execute(&db, &parse_admin("FLUSH FULL")).expect("test: flush full");
    assert!(msg.contains("no-op"));
}

#[test]
fn test_flush_existing_collection_ok() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    let msg = execute(&db, &parse_admin("FLUSH docs")).expect("test: flush docs");
    assert!(msg.contains("no-op"));
}

#[test]
fn test_flush_missing_collection_errors() {
    let db = DatabaseInner::new();
    let err = execute(&db, &parse_admin("FLUSH ghost"));
    assert!(err.is_err());
}
