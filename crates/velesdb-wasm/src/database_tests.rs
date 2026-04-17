//! Unit tests for `WasmDatabase` collection lifecycle.
//!
//! Tests exercise `DatabaseInner` directly (returns `String` errors) so they
//! run on the native host target without requiring a WASM runtime.

use super::*;

// =========================================================================
// Nominal path tests
// =========================================================================

#[test]
fn test_new_database_is_empty() {
    let db = DatabaseInner::new();
    assert_eq!(db.collection_count(), 0);
}

#[test]
fn test_create_collection_increases_count() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create should succeed");
    assert_eq!(db.collection_count(), 1);
}

#[test]
fn test_create_multiple_collections() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: first create");
    db.create_collection("images", 128, "euclidean")
        .expect("test: second create");
    assert_eq!(db.collection_count(), 2);
}

#[test]
fn test_delete_collection_decreases_count() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create");
    db.delete_collection("docs")
        .expect("test: delete should succeed");
    assert_eq!(db.collection_count(), 0);
}

#[test]
fn test_get_shared_store_returns_handle() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create");
    let store = db
        .get_shared_store("docs")
        .expect("test: get should succeed");
    let borrowed = store.borrow();
    assert_eq!(borrowed.dimension(), 4);
    assert!(borrowed.is_empty());
}

#[test]
fn test_handle_insert_visible_across_handles() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create");
    let handle = db.get_shared_store("docs").expect("test: get");
    handle
        .borrow_mut()
        .insert(1, &[1.0, 0.0, 0.0, 0.0])
        .expect("test: insert");
    assert_eq!(handle.borrow().len(), 1);

    // A second handle to the same collection sees the insert
    let handle2 = db.get_shared_store("docs").expect("test: get again");
    assert_eq!(handle2.borrow().len(), 1);
}

#[test]
fn test_handle_remove_works() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create");
    let handle = db.get_shared_store("docs").expect("test: get");
    handle
        .borrow_mut()
        .insert(1, &[1.0, 0.0, 0.0, 0.0])
        .expect("test: insert");
    assert!(handle.borrow_mut().remove(1));
    assert!(handle.borrow().is_empty());
}

#[test]
fn test_all_supported_metrics() {
    let mut db = DatabaseInner::new();
    for metric in ["cosine", "euclidean", "dot", "hamming", "jaccard"] {
        let name = format!("coll_{metric}");
        db.create_collection(&name, 8, metric)
            .unwrap_or_else(|_| panic!("test: metric '{metric}' should be valid"));
    }
    assert_eq!(db.collection_count(), 5);
}

#[test]
fn test_list_collection_names() {
    let mut db = DatabaseInner::new();
    db.create_collection("alpha", 4, "cosine")
        .expect("test: create alpha");
    db.create_collection("beta", 4, "cosine")
        .expect("test: create beta");
    let mut names = db.collection_names();
    names.sort();
    assert_eq!(names, vec!["alpha", "beta"]);
}

// =========================================================================
// Error / edge-case tests
// =========================================================================

#[test]
fn test_create_duplicate_returns_error() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: first create");
    let err = db.create_collection("docs", 4, "cosine");
    assert!(err.is_err(), "duplicate create should fail");
    let msg = err.unwrap_err();
    assert!(
        msg.contains("already exists"),
        "error should mention 'already exists', got: {msg}"
    );
}

#[test]
fn test_create_with_invalid_metric_returns_error() {
    let mut db = DatabaseInner::new();
    let err = db.create_collection("bad", 4, "unknown_metric");
    assert!(err.is_err(), "invalid metric should fail");
    assert_eq!(
        db.collection_count(),
        0,
        "no collection should be added on error"
    );
}

#[test]
fn test_delete_missing_returns_error() {
    let mut db = DatabaseInner::new();
    let err = db.delete_collection("ghost");
    assert!(err.is_err(), "delete nonexistent should fail");
    let msg = err.unwrap_err();
    assert!(
        msg.contains("not found"),
        "error should mention 'not found', got: {msg}"
    );
}

#[test]
fn test_get_missing_returns_error() {
    let db = DatabaseInner::new();
    let result = db.get_shared_store("ghost");
    assert!(result.is_err(), "get nonexistent should fail");
    // Cannot use unwrap_err() because SharedStore (Ok type) lacks Debug.
    let msg = match result {
        Err(e) => e,
        Ok(_) => unreachable!("already asserted is_err"),
    };
    assert!(
        msg.contains("not found"),
        "error should mention 'not found', got: {msg}"
    );
}

#[test]
fn test_create_then_delete_then_create_same_name() {
    let mut db = DatabaseInner::new();
    db.create_collection("reuse", 4, "cosine")
        .expect("test: first create");
    db.delete_collection("reuse").expect("test: delete");
    db.create_collection("reuse", 8, "euclidean")
        .expect("test: re-create with different params");
    let store = db.get_shared_store("reuse").expect("test: get");
    assert_eq!(store.borrow().dimension(), 8);
}

// Note: dimension-mismatch tests on VectorStore::insert use JsValue::from_str
// internally, which panics on non-wasm32. That path is covered by VectorStore's
// own wasm-bindgen-test suite. Here we only verify the DatabaseInner contract.

#[test]
fn test_handle_insert_correct_dimension_succeeds() {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create");
    let store = db.get_shared_store("docs").expect("test: get");
    store
        .borrow_mut()
        .insert(1, &[1.0, 0.0, 0.0, 0.0])
        .expect("test: insert with correct dimension");
    assert_eq!(store.borrow().len(), 1);
}

#[test]
fn test_create_zero_dimension_succeeds() {
    // Metadata-only collections have dimension 0
    let mut db = DatabaseInner::new();
    db.create_collection("meta", 0, "cosine")
        .expect("test: zero-dim should succeed");
    assert_eq!(db.collection_count(), 1);
}

#[test]
fn test_delete_does_not_affect_other_collections() {
    let mut db = DatabaseInner::new();
    db.create_collection("a", 4, "cosine")
        .expect("test: create a");
    db.create_collection("b", 8, "euclidean")
        .expect("test: create b");
    db.delete_collection("a").expect("test: delete a");
    assert_eq!(db.collection_count(), 1);
    let store_b = db.get_shared_store("b").expect("test: b still exists");
    assert_eq!(store_b.borrow().dimension(), 8);
}

#[test]
fn test_wasm_database_default_trait() {
    let db = WasmDatabase::default();
    assert_eq!(db.inner.collection_count(), 0);
}

// =========================================================================
// Metadata collection tests (S4-13)
// =========================================================================

#[test]
fn test_create_metadata_collection_succeeds() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs")
        .expect("test: metadata create should succeed");
    assert_eq!(db.collection_count(), 1);
    let store = db
        .get_shared_store("docs")
        .expect("test: metadata store retrievable");
    assert_eq!(store.borrow().dimension(), 0);
    assert!(store.borrow().is_metadata_only());
}

#[test]
fn test_create_metadata_collection_duplicate_fails() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs")
        .expect("test: first create");
    let err = db.create_metadata_collection("docs");
    assert!(err.is_err(), "duplicate metadata create should fail");
}

#[test]
fn test_contains_reports_existing_collection() {
    let mut db = DatabaseInner::new();
    assert!(!db.contains("docs"));
    db.create_metadata_collection("docs").expect("test: create");
    assert!(db.contains("docs"));
    assert!(!db.contains("ghost"));
}

// =========================================================================
// Graph store lifecycle (Devin Review PR #594 finding #3)
// =========================================================================
//
// Collections and their associated graph stores share a namespace. Dropping
// a collection must also drop any graph store so that recreating a
// collection with the same name cannot resurface ghost nodes/edges.

#[test]
fn test_delete_collection_also_drops_associated_graph_store() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("g").expect("test: create");

    // Populate the graph store.
    let g = db.graph_store("g");
    g.borrow_mut().upsert_node(1, None, vec!["Person".into()]);
    g.borrow_mut()
        .insert_edge(None, 1, 2, "KNOWS".into(), None)
        .expect("test: insert edge");
    drop(g);
    assert!(
        db.has_graph_store("g"),
        "graph store should exist after seed"
    );

    // Dropping the collection must drop its graph store.
    db.delete_collection("g").expect("test: delete");
    assert!(
        !db.has_graph_store("g"),
        "graph store must be removed when collection is dropped"
    );

    // Recreating with the same name must not resurface the old graph data.
    db.create_metadata_collection("g").expect("test: recreate");
    assert!(
        !db.has_graph_store("g"),
        "lazy creation only: fresh collection has no graph store yet"
    );
    let g2 = db.graph_store("g");
    assert!(
        g2.borrow().all_node_ids().is_empty(),
        "newly-provisioned graph store must be empty"
    );
    assert!(
        g2.borrow().edges().is_empty(),
        "newly-provisioned graph store must have no edges"
    );
}

#[test]
fn test_delete_collection_without_graph_store_does_not_panic() {
    // Negative: a collection that never had any graph DML must still be
    // droppable without touching `self.graphs`.
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("no_graph")
        .expect("test: create");
    assert!(!db.has_graph_store("no_graph"));
    db.delete_collection("no_graph").expect("test: delete ok");
}

#[test]
fn test_clear_graph_store_empties_existing_graph() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("g").expect("test: create");
    let g = db.graph_store("g");
    g.borrow_mut().upsert_node(1, None, vec![]);
    g.borrow_mut()
        .insert_edge(None, 1, 2, "R".into(), None)
        .expect("test: insert edge");
    drop(g);

    db.clear_graph_store("g");

    assert!(db.has_graph_store("g"), "clear must not remove the store");
    let g2 = db.get_graph_store("g").expect("test: still registered");
    assert!(g2.borrow().all_node_ids().is_empty());
    assert!(g2.borrow().edges().is_empty());
}

#[test]
fn test_clear_graph_store_no_op_on_absent_name() {
    let mut db = DatabaseInner::new();
    // Must not panic even though no graph store has ever been created.
    db.clear_graph_store("ghost");
    assert!(!db.has_graph_store("ghost"));
}

#[test]
fn test_collection_summaries_lists_vector_and_metadata() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: vector");
    db.create_metadata_collection("docs")
        .expect("test: metadata");
    let mut summaries = db.collection_summaries();
    summaries.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0], ("docs".to_string(), 0, true));
    assert_eq!(summaries[1], ("vecs".to_string(), 4, false));
}
