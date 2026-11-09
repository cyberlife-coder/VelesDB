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

#[test]
fn scan_all_refuses_over_cardinality_limit() {
    // The WHERE-scan guardrail (#2679): the 100_001st materialized row is
    // refused with core's `[VELES-027]` tag inside the executor's String
    // channel. `payloads` stays empty — `scan_all` tolerates missing
    // payloads via `get(idx)` — so the fixture is just the id list.
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("big").expect("test: create");
    let store = db.get_shared_store("big").expect("test: store");
    {
        let mut b = store.borrow_mut();
        b.ids
            .extend(0..=crate::guardrails::MAX_INTERMEDIATE_CARDINALITY as u64);
    }
    let err = scan_all(&db, "big", None, &Params::new()).unwrap_err();
    assert!(err.contains("VELES-027"), "missing guardrail code: {err}");
}

#[test]
fn scan_all_accepts_at_cardinality_limit() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("edge").expect("test: create");
    let store = db.get_shared_store("edge").expect("test: store");
    {
        let mut b = store.borrow_mut();
        b.ids
            .extend(0..crate::guardrails::MAX_INTERMEDIATE_CARDINALITY as u64);
    }
    let rows = scan_all(&db, "edge", None, &Params::new()).expect("test: scan");
    assert_eq!(rows.len(), crate::guardrails::MAX_INTERMEDIATE_CARDINALITY);
}
