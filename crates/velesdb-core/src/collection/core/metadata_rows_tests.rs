//! A metadata collection's rows are its payloads (`RowUniverse::Payloads`),
//! written by one rule on `upsert` and `upsert_bulk` — the rule a graph
//! writes its rows without a vector by — sparse vectors included: a row
//! carrying only sparse vectors is a row (with an empty payload), a delete
//! removes its sparse postings, and a sparse search reaches rows only.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::collection::types::Collection;
use crate::index::sparse::SparseVector;
use crate::point::Point;

fn metadata(dir: &Path) -> Collection {
    Collection::create_metadata_only(PathBuf::from(dir), "m").expect("test: metadata collection")
}

fn sparse(weight: f32) -> SparseVector {
    SparseVector::new(vec![(1, weight), (2, weight)])
}

fn row(id: u64, payload: Option<Value>, weight: Option<f32>) -> Point {
    Point {
        id,
        vector: Vec::new(),
        payload,
        sparse_vectors: weight.map(|w| BTreeMap::from([(String::new(), sparse(w))])),
    }
}

/// The ids a `SPARSE_NEAR` returns, best first.
fn sparse_hits(collection: &Collection, limit: usize) -> Vec<u64> {
    let params = HashMap::from([("s".to_string(), json!({"1": 1.0, "2": 1.0}))]);
    collection
        .execute_query_str(
            &format!("SELECT * FROM m WHERE vector SPARSE_NEAR $s LIMIT {limit}"),
            &params,
        )
        .expect("test: SPARSE_NEAR")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

fn payload(collection: &Collection, id: u64) -> Option<Value> {
    collection.get(&[id])[0]
        .as_ref()
        .and_then(|p| p.payload.clone())
}

#[test]
fn upsert_stores_the_sparse_vectors_upsert_bulk_stores() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let upserted = metadata(&dir.path().join("a"));
    let bulked = metadata(&dir.path().join("b"));
    let rows = [row(1, Some(json!({"a": 1})), Some(1.0))];

    upserted.upsert(rows.clone()).expect("test: upsert");
    bulked.upsert_bulk(&rows).expect("test: bulk");

    assert_eq!(sparse_hits(&bulked, 10), [1], "bulk");
    assert_eq!(sparse_hits(&upserted, 10), [1], "upsert as bulk");
    assert_eq!(payload(&upserted, 1), payload(&bulked, 1));
}

#[test]
fn upsert_replaces_sparse_vectors_and_bulk_adds_them() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = metadata(dir.path());
    rows.upsert([row(1, Some(json!({"a": 1})), Some(1.0))])
        .expect("test: upsert");
    rows.upsert([row(1, Some(json!({"a": 2})), None)])
        .expect("test: upsert without sparse vectors");
    assert!(sparse_hits(&rows, 10).is_empty(), "an upsert replaces them");

    rows.upsert_bulk(&[row(1, None, Some(1.0))])
        .expect("test: bulk");
    assert_eq!(sparse_hits(&rows, 10), [1], "a bulk write adds them");
    assert_eq!(
        payload(&rows, 1),
        Some(json!({"a": 2})),
        "and keeps the payload"
    );
}

#[test]
fn a_row_carrying_only_sparse_vectors_is_a_row() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = metadata(dir.path());

    assert_eq!(
        rows.upsert_bulk(&[row(1, None, Some(1.0))]).expect("test"),
        1
    );
    rows.upsert([row(2, None, Some(1.0))])
        .expect("test: upsert");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows.all_ids(), [1, 2]);
    assert_eq!(payload(&rows, 1), Some(json!({})), "an empty payload");
    assert_eq!(payload(&rows, 2), Some(json!({})));
    assert_eq!(sparse_hits(&rows, 10).len(), 2);
    assert_eq!(rows.delete_counted(&[1, 2]).expect("test: delete"), 2);
    assert!(rows.is_empty());
}

#[test]
fn a_delete_removes_the_sparse_postings_of_a_row() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = metadata(dir.path());
    rows.upsert([
        row(1, Some(json!({"a": 1})), Some(1.0)),
        row(2, Some(json!({"a": 2})), Some(2.0)),
    ])
    .expect("test: upsert");

    rows.delete(&[2]).expect("test: delete");
    assert_eq!(sparse_hits(&rows, 10), [1], "the deleted row is not found");

    rows.flush_full().expect("test: flush");
    drop(rows);
    let reopened = Collection::open(PathBuf::from(dir.path())).expect("test: reopen");
    assert_eq!(sparse_hits(&reopened, 10), [1], "nor after a reopen");
}

/// A sparse posting an earlier version stored for an id without a payload
/// (a bulk sparse-only point it accepted without making it a row) is never
/// ranked, so it cannot take a row's place in the `LIMIT`.
#[test]
fn a_sparse_search_ranks_rows_only() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = metadata(dir.path());
    rows.apply_sparse_batch_bulk(&Collection::collect_sparse_batch(&[row(
        7,
        None,
        Some(10.0),
    )]))
    .expect("test: a posting for no row");
    rows.upsert([row(1, Some(json!({"a": 1})), Some(1.0))])
        .expect("test: upsert");

    assert_eq!(sparse_hits(&rows, 1), [1]);
}

/// `upsert` replaces a row: a row carrying no payload stores an empty one,
/// as on a graph. It used to delete the row.
#[test]
fn upsert_of_a_row_without_a_payload_stores_an_empty_one() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = metadata(dir.path());
    rows.upsert([row(1, Some(json!({"a": 1})), None)])
        .expect("test: upsert");

    rows.upsert([row(1, None, None)]).expect("test: upsert");

    assert_eq!(rows.len(), 1);
    assert_eq!(payload(&rows, 1), Some(json!({})));
}

/// A refused batch writes nothing: an invalid sparse index name reaches no
/// store, on either path.
#[test]
fn an_invalid_sparse_name_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = metadata(dir.path());
    let hostile = Point {
        id: 2,
        vector: Vec::new(),
        payload: Some(json!({"a": 2})),
        sparse_vectors: Some(BTreeMap::from([("../x".to_string(), sparse(1.0))])),
    };
    let batch = [row(1, Some(json!({"a": 1})), None), hostile];

    assert!(rows.upsert(batch.clone()).is_err(), "upsert");
    assert!(
        rows.upsert_metadata(batch.clone()).is_err(),
        "upsert_metadata"
    );
    assert!(rows.upsert_bulk(&batch).is_err(), "upsert_bulk");
    assert!(rows.is_empty());
}

/// `upsert_metadata` is a write surface like any other: a vector is refused
/// with `VELES-014`, as `upsert` refuses it, before anything is written. It
/// used to be dropped without a word.
#[test]
fn upsert_metadata_refuses_a_vector() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let rows = metadata(dir.path());
    let refused = rows.upsert_metadata([
        row(1, Some(json!({"a": 1})), None),
        Point::new(2, vec![1.0], Some(json!({"a": 2}))),
    ]);
    assert_eq!(refused.map_err(|e| e.code()), Err("VELES-014"));
    assert!(rows.is_empty());
}
