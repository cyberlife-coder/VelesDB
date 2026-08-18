use super::*;
use crate::database::DatabaseInner;
use velesdb_core::velesql::Parser;

fn seed(db: &mut DatabaseInner) {
    db.create_metadata_collection("t").expect("test: create");
    let store = db.get_shared_store("t").expect("test: store");
    let mut b = store.borrow_mut();
    for (id, cat) in [(1u64, "a"), (2, "b"), (3, "a")] {
        b.ids.push(id);
        b.payloads.push(Some(serde_json::json!({"cat": cat})));
    }
}

#[test]
fn test_scan_all_no_where() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let rows = scan_all(&db, "t", None, &Params::new()).expect("test: scan");
    assert_eq!(rows.len(), 3);
}

#[test]
fn test_scan_all_with_where() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let q = Parser::parse("SELECT * FROM t WHERE cat = 'a'").expect("test: parse");
    let rows =
        scan_all(&db, "t", q.select.where_clause.as_ref(), &Params::new()).expect("test: scan");
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_scan_missing_collection_errors() {
    let db = DatabaseInner::new();
    let err = scan_all(&db, "ghost", None, &Params::new());
    assert!(err.is_err());
}
