//! Native coverage tests for VelesDB 3.0.0 WASM paths.
//!
//! These exercise the native-callable Rust bodies behind the
//! `#[wasm_bindgen]` surface without touching the `JsValue` serialization
//! paths (which panic off-`wasm32`). They target previously-uncovered
//! branches in `vector_store.rs`, `sparse.rs`, `store_search` delegation,
//! `database.rs` (`WasmCollectionHandle`), and `velesql_match.rs`
//! (`enrich_row` skip branches).

use velesdb_core::velesql::{InsertEdgeStatement, InsertNodeStatement, Parser, Query};

use crate::database::{DatabaseInner, WasmDatabase};
use crate::sparse::SparseIndex;
use crate::store_insert::insert_with_payload;
use crate::store_new::create_store;
use crate::velesql_graph::{execute_match, insert_edge, insert_node};
use crate::velesql_value::Params;
use crate::{DistanceMetric, StorageMode};

// =========================================================================
// SparseIndex::search_scored — native scoring kernel branches
// (sparse.rs ~L166-202; covers the k==0 and length-mismatch guards plus the
//  DAAT accumulation + top-k ranking path)
// =========================================================================

#[test]
fn test_sparse_index_search_scored_ranks_by_dot_product() {
    let mut index = SparseIndex::new();
    index
        .insert(1, &[10, 20, 30], &[1.0, 0.5, 0.3])
        .expect("test: insert 1");
    index
        .insert(2, &[10, 40], &[0.8, 1.2])
        .expect("test: insert 2");
    index
        .insert(3, &[20, 30, 50], &[0.9, 0.7, 0.4])
        .expect("test: insert 3");
    index
        .insert(4, &[10, 20], &[0.3, 1.5])
        .expect("test: insert 4");

    // query = {10: 1.0, 20: 1.0}:
    //   doc 4 -> 0.3 + 1.5 = 1.8 (top)
    //   doc 1 -> 1.0 + 0.5 = 1.5
    //   doc 3 -> 0.9
    //   doc 2 -> 0.8
    let results = index
        .search_scored(&[10, 20], &[1.0, 1.0], 10)
        .expect("test: search_scored on populated index");
    assert_eq!(results.len(), 4, "all four docs touch the query terms");
    assert_eq!(results[0].doc_id(), 4, "doc 4 (1.8) ranks first");
    assert_eq!(results[1].doc_id(), 1, "doc 1 (1.5) ranks second");
}

#[test]
fn test_sparse_index_search_scored_truncates_to_k() {
    let mut index = SparseIndex::new();
    index.insert(1, &[1], &[3.0]).expect("test: insert 1");
    index.insert(2, &[1], &[2.0]).expect("test: insert 2");
    index.insert(3, &[1], &[1.0]).expect("test: insert 3");

    let results = index
        .search_scored(&[1], &[1.0], 2)
        .expect("test: search_scored truncates");
    assert_eq!(results.len(), 2, "k=2 caps the result list");
    assert_eq!(results[0].doc_id(), 1, "highest weight first");
    assert_eq!(results[1].doc_id(), 2, "second highest next");
}

#[test]
fn test_sparse_index_search_scored_k_zero_returns_empty() {
    // sparse.rs L179-181: k == 0 short-circuits to an empty result vector
    // before any accumulation work.
    let mut index = SparseIndex::new();
    index.insert(1, &[10], &[1.0]).expect("test: insert");
    let results = index
        .search_scored(&[10], &[1.0], 0)
        .expect("test: k=0 is a valid empty query");
    assert!(results.is_empty(), "k=0 yields no results");
}

#[test]
fn test_sparse_index_search_scored_length_mismatch_errors() {
    // sparse.rs L172-178: query indices/values length disagreement is
    // rejected before scoring with a descriptive error.
    let index = SparseIndex::new();
    let err = index
        .search_scored(&[10, 20], &[1.0], 5)
        .expect_err("test: 2 indices vs 1 value must error");
    assert!(
        err.contains("indices/values length mismatch"),
        "error should describe the mismatch, got: {err}"
    );
}

#[test]
fn test_sparse_index_search_scored_no_matching_terms_is_empty() {
    // A query whose terms are absent from every posting list accumulates
    // nothing and returns an empty ranking (no error).
    let mut index = SparseIndex::new();
    index
        .insert(1, &[10, 20], &[1.0, 1.0])
        .expect("test: insert");
    let results = index
        .search_scored(&[999], &[1.0], 5)
        .expect("test: unknown term is a valid empty query");
    assert!(results.is_empty(), "absent query term matches nothing");
}

// =========================================================================
// VectorStore::search_sparse_scored — delegate behind search_sparse
// (vector_store.rs ~L366-367 / L628-639; exercises the k==0 and
//  length-mismatch propagation from the underlying kernel)
// =========================================================================

#[test]
fn test_vector_store_search_sparse_scored_k_zero() {
    let mut store = create_store(4, DistanceMetric::Cosine, StorageMode::Full);
    store
        .sparse_insert(1, &[10, 20], &[1.0, 1.0])
        .expect("test: sparse_insert");
    let results = store
        .search_sparse_scored(&[10], &[1.0], 0)
        .expect("test: k=0 is valid");
    assert!(
        results.is_empty(),
        "k=0 yields no results through the store"
    );
}

#[test]
fn test_vector_store_search_sparse_scored_mismatch_propagates() {
    let mut store = create_store(4, DistanceMetric::Cosine, StorageMode::Full);
    store
        .sparse_insert(1, &[10, 20], &[1.0, 1.0])
        .expect("test: sparse_insert");
    let err = store
        .search_sparse_scored(&[10, 20, 30], &[1.0], 5)
        .expect_err("test: 3 indices vs 1 value must error through the store");
    assert!(
        err.contains("length mismatch"),
        "store should surface the kernel's mismatch error, got: {err}"
    );
}

// =========================================================================
// VectorStore::insert_batch_raw — wasm-bindgen method, native happy path
// (vector_store.rs ~L609-617; success branch never builds a JsValue)
// =========================================================================

#[test]
fn test_vector_store_insert_batch_raw_method_happy_path() {
    let mut store = create_store(3, DistanceMetric::Euclidean, StorageMode::Full);
    let ids = [7u64, 8, 9];
    let vectors = [
        1.0, 0.0, 0.0, // id 7
        0.0, 1.0, 0.0, // id 8
        0.0, 0.0, 1.0, // id 9
    ];
    store
        .insert_batch_raw(&ids, &vectors, 3)
        .expect("test: raw bulk insert through the wasm method body");
    assert_eq!(store.ids, vec![7, 8, 9]);
    assert_eq!(store.data.len(), 9);
    assert_eq!(&store.data[3..6], &[0.0, 1.0, 0.0]);
}

// =========================================================================
// WasmCollectionHandle::insert_batch_raw — handle delegate, native happy path
// (database.rs ~L416-425; resolves a handle from WasmDatabase and bulk-inserts)
// =========================================================================

#[test]
fn test_collection_handle_insert_batch_raw_happy_path() {
    let mut db = WasmDatabase::new();
    db.create_collection("vecs", 2, "cosine")
        .expect("test: create collection (native success branch)");
    let handle = db
        .get_collection("vecs")
        .expect("test: get_collection success branch builds no JsValue");

    handle
        .insert_batch_raw(&[100u64, 101], &[1.0, 0.0, 0.0, 1.0], 2)
        .expect("test: handle bulk insert through borrow_mut");
    assert_eq!(handle.len(), 2, "both rows landed in the shared store");

    // The insert is visible through a freshly resolved handle (shared Rc).
    let handle2 = db.get_collection("vecs").expect("test: second handle");
    assert_eq!(handle2.len(), 2, "second handle sees the same store");
    assert_eq!(handle2.dimension(), 2);
    assert!(!handle2.is_empty());
}

// =========================================================================
// enrich_row — cross-collection MATCH enrichment skip branches
// (velesql_match.rs ~L66-67: payload_for_id missing / non-object → continue)
// =========================================================================

fn parse_match(sql: &str) -> Query {
    Parser::parse(sql).expect("test: parse")
}

fn seed_has_edge(db: &mut DatabaseInner) {
    for (id, name, labels) in [(1u64, "Alice", vec!["Person"]), (2, "Bob", vec!["Profile"])] {
        let stmt = InsertNodeStatement {
            collection: "graph".to_string(),
            node_id: id,
            payload: serde_json::json!({"name": name, "labels": labels}),
        };
        insert_node(db, &stmt).expect("test: insert node");
    }
    let edge = InsertEdgeStatement {
        collection: "graph".to_string(),
        edge_id: None,
        source: 1,
        target: 2,
        label: "HAS".to_string(),
        properties: Vec::new(),
    };
    insert_edge(db, &edge, &Params::new()).expect("test: insert edge");
}

#[test]
fn test_enrich_row_skips_when_referenced_id_absent() {
    // The @collection exists (cross-ref resolves), but it holds NO payload
    // for the matched node id. payload_for_id(id) returns None → the
    // enrichment loop hits `continue` and the row stays un-enriched.
    let mut db = DatabaseInner::new();
    // Referenced collection exists but is empty (no id=2 payload).
    db.create_metadata_collection("profiles")
        .expect("test: create profiles");
    seed_has_edge(&mut db);

    let q = parse_match("MATCH (a:Person)-[:HAS]->(b:Profile@profiles) RETURN a, b LIMIT 10");
    let rows = execute_match(&mut db, &q, &Params::new()).expect("test: match still succeeds");
    assert_eq!(rows.len(), 1);
    let data: serde_json::Value =
        serde_json::from_str(rows[0].data_json_ref()).expect("test: row json");
    // Graph identity is intact; no cross fields were merged.
    assert_eq!(data["b"]["name"], serde_json::json!("Bob"));
    assert_eq!(data["b"]["id"], serde_json::json!(2));
    assert!(
        data["b"].get("email").is_none(),
        "absent referenced payload must not enrich the row"
    );
}

#[test]
fn test_enrich_row_skips_when_referenced_payload_not_object() {
    // The @collection holds a payload for id=2, but it is a JSON STRING, not
    // an object. The `Some(Value::Object(..))` else-branch fires → continue,
    // leaving the row un-enriched without panicking.
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("profiles")
        .expect("test: create profiles");
    {
        let store = db
            .get_shared_store("profiles")
            .expect("test: profiles store");
        insert_with_payload(
            &mut store.borrow_mut(),
            2,
            &[],
            Some(serde_json::json!("scalar-not-an-object")),
        );
    }
    seed_has_edge(&mut db);

    let q = parse_match("MATCH (a:Person)-[:HAS]->(b:Profile@profiles) RETURN a, b LIMIT 10");
    let rows = execute_match(&mut db, &q, &Params::new()).expect("test: match still succeeds");
    assert_eq!(rows.len(), 1);
    let data: serde_json::Value =
        serde_json::from_str(rows[0].data_json_ref()).expect("test: row json");
    assert_eq!(data["b"]["name"], serde_json::json!("Bob"));
    assert_eq!(data["b"]["id"], serde_json::json!(2));
    // A non-object referenced payload merges nothing.
    assert!(data["b"].as_object().expect("test: b is object").len() <= 3);
}
