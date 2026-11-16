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

/// Removing a row drops its sparse postings too — `clear()` already did,
/// for the reason its doc gives: they would reference a row that is gone.
#[test]
fn given_a_removed_row_when_searching_sparse_then_it_is_not_returned() {
    let mut store = store_with_two_points();
    store
        .sparse_insert(1, &[7], &[1.0])
        .expect("test: sparse 1");
    store
        .sparse_insert(2, &[7], &[0.5])
        .expect("test: sparse 2");
    assert!(store.remove(1));
    let hits = store
        .state
        .borrow()
        .search_sparse_scored(&[7], &[1.0], 10)
        .expect("test: sparse search");
    let ids: Vec<u64> = hits
        .iter()
        .map(crate::sparse::SparseSearchResult::doc_id)
        .collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn given_a_velesql_delete_when_searching_sparse_then_the_row_is_not_returned() {
    let mut store = store_with_two_points();
    store
        .sparse_insert(1, &[7], &[1.0])
        .expect("test: sparse 1");
    let mut db = WasmDatabase::new();
    db.attach_store("docs", &store).expect("test: attach");
    execute(db.inner_mut(), "DELETE FROM docs WHERE id = 1", None).expect("test: delete");
    let hits = store
        .state
        .borrow()
        .search_sparse_scored(&[7], &[1.0], 10)
        .expect("test: sparse search");
    assert!(hits.is_empty(), "the deleted row's postings are gone");
}

/// `CREATE COLLECTION … WITH (storage = …)` reads the mode as core does,
/// refusing a name core refuses; it used to be dropped (every collection Full).
#[test]
fn given_create_collection_with_a_storage_mode_then_it_is_honoured_or_refused() {
    let mut db = WasmDatabase::new();
    execute(
        db.inner_mut(),
        "CREATE COLLECTION q (dimension = 4, metric = 'cosine') WITH (storage = 'sq8')",
        None,
    )
    .expect("test: sq8 create");
    let store = db.inner_mut().get_shared_store("q").expect("test: store");
    assert_eq!(store.borrow().storage_mode(), "sq8");
    assert!(execute(
        db.inner_mut(),
        "CREATE COLLECTION b (dimension = 4, metric = 'cosine') WITH (storage = 'bogus')",
        None,
    )
    .is_err());
}

/// The preset parser runs without a store: the SDK validates a `quality`
/// with it ahead of a search, now that a metadata-only store refuses one.
#[test]
fn given_a_valid_preset_when_validating_then_no_store_is_needed() {
    for preset in [
        "fast",
        "balanced",
        "accurate",
        "perfect",
        "autotune",
        "custom:128",
    ] {
        assert!(
            VectorStore::validate_search_quality(preset).is_ok(),
            "{preset}"
        );
    }
}
