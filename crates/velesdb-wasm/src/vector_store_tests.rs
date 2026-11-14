//! BDD tests for sharing one `VectorStore` between JavaScript and a database.
//!
//! Before: a store created from JavaScript owned its data, so a
//! `WasmDatabase` could not run VelesQL over it — the TypeScript SDK's
//! browser backend therefore answered only pure `NEAR` searches. Attaching a
//! store must share it both ways, with no copy.

use crate::database::{DatabaseInner, WasmDatabase};
use crate::velesql_exec::execute;
use crate::VectorStore;

fn store_with_two_points() -> VectorStore {
    let mut store = VectorStore::new(2, "cosine").expect("test: store");
    store.insert(1, &[1.0, 0.0]).expect("test: insert 1");
    store.insert(2, &[0.0, 1.0]).expect("test: insert 2");
    store
}

fn ids(db: &mut DatabaseInner, sql: &str) -> Vec<u64> {
    let result = execute(db, sql, None).expect("test: query");
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&result.rows_json()).expect("test: rows json");
    let mut ids: Vec<u64> = rows
        .iter()
        .map(|row| row["id"].as_u64().expect("test: id"))
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn given_an_attached_store_when_querying_the_database_then_it_sees_the_handles_points() {
    let store = store_with_two_points();
    let mut db = WasmDatabase::new();

    db.attach_store("docs", &store).expect("test: attach");

    assert_eq!(
        ids(db.inner_mut(), "SELECT * FROM docs LIMIT 10"),
        vec![1, 2]
    );
}

#[test]
fn given_an_attached_store_when_the_database_writes_then_the_handle_sees_it() {
    let store = store_with_two_points();
    let mut db = WasmDatabase::new();
    db.attach_store("docs", &store).expect("test: attach");

    execute(
        db.inner_mut(),
        "INSERT INTO docs (id, vector) VALUES (3, $v)",
        Some(r#"{"v": [0.5, 0.5]}"#),
    )
    .expect("test: insert through VelesQL");

    assert_eq!(
        store.len(),
        3,
        "the handle and the database share one store"
    );
}

#[test]
fn given_an_attached_store_when_the_handle_writes_afterwards_then_the_database_sees_it() {
    let mut store = store_with_two_points();
    let mut db = WasmDatabase::new();
    db.attach_store("docs", &store).expect("test: attach");

    store
        .insert(7, &[1.0, 1.0])
        .expect("test: insert after attach");

    assert_eq!(
        ids(db.inner_mut(), "SELECT * FROM docs LIMIT 10"),
        vec![1, 2, 7]
    );
}

#[test]
fn given_a_taken_name_when_attaching_then_it_is_refused_and_the_first_store_kept() {
    let first = store_with_two_points();
    let second = VectorStore::new(2, "cosine").expect("test: store");
    let mut db = WasmDatabase::new();
    db.attach_store("docs", &first).expect("test: first attach");

    assert!(db
        .inner_mut()
        .attach_shared_store("docs", second.shared_state())
        .is_err());
    assert_eq!(
        ids(db.inner_mut(), "SELECT * FROM docs LIMIT 10"),
        vec![1, 2]
    );
}
