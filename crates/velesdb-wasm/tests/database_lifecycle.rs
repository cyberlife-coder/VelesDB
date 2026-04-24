//! Integration tests for `WasmDatabase` collection lifecycle APIs.
//!
//! These run on the host target (not wasm32) to verify public API contracts.
//! They use the `WasmDatabase` public surface, not the internal `DatabaseInner`.

#![cfg(not(target_arch = "wasm32"))]

use velesdb_wasm::WasmDatabase;

#[test]
fn wasm_database_create_and_count() {
    let mut db = WasmDatabase::new();
    assert_eq!(db.collection_count(), 0);
    db.create_collection("test", 4, "cosine")
        .expect("test: create should succeed");
    assert_eq!(db.collection_count(), 1);
}

#[test]
fn wasm_database_delete_and_count() {
    let mut db = WasmDatabase::new();
    db.create_collection("test", 4, "cosine")
        .expect("test: create");
    db.delete_collection("test")
        .expect("test: delete should succeed");
    assert_eq!(db.collection_count(), 0);
}

#[test]
fn wasm_database_get_collection_dimension() {
    let mut db = WasmDatabase::new();
    db.create_collection("embeddings", 768, "cosine")
        .expect("test: create");
    let handle = db
        .get_collection("embeddings")
        .expect("test: get should succeed");
    assert_eq!(handle.dimension(), 768);
    assert!(handle.is_empty());
}

#[test]
fn wasm_database_handle_insert_and_len() {
    let mut db = WasmDatabase::new();
    db.create_collection("v", 3, "euclidean")
        .expect("test: create");
    let handle = db.get_collection("v").expect("test: get");
    handle.insert(42, &[1.0, 2.0, 3.0]).expect("test: insert");
    assert_eq!(handle.len(), 1);
    assert!(!handle.is_empty());
}

#[test]
fn wasm_database_shared_state_between_handles() {
    let mut db = WasmDatabase::new();
    db.create_collection("shared", 2, "cosine")
        .expect("test: create");

    let h1 = db.get_collection("shared").expect("test: get h1");
    h1.insert(1, &[0.5, 0.5]).expect("test: insert via h1");

    let h2 = db.get_collection("shared").expect("test: get h2");
    assert_eq!(h2.len(), 1, "h2 should see h1's insert");
}

#[test]
fn wasm_database_handle_remove() {
    let mut db = WasmDatabase::new();
    db.create_collection("rm", 2, "dot").expect("test: create");
    let handle = db.get_collection("rm").expect("test: get");
    handle.insert(10, &[1.0, 0.0]).expect("test: insert");
    assert!(handle.remove(10), "remove existing should return true");
    assert!(!handle.remove(10), "remove missing should return false");
    assert!(handle.is_empty());
}
