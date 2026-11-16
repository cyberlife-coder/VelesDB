//! A point's sparse vector is what its last upsert said, and nothing else.
//!
//! Upsert replaces the dense vector and the payload; the sparse index kept
//! every term the point ever had instead, so a re-upserted document still
//! matched on words it no longer contains. Non-finite weights reached the
//! index too (only the REST request parser refused them), and a NaN weight
//! poisons every score it touches.

#![cfg(feature = "persistence")]

use std::collections::BTreeMap;

use tempfile::TempDir;
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{Database, DistanceMetric, Point, VectorCollection};

fn docs() -> (TempDir, Database, VectorCollection) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("docs", 2, DistanceMetric::Cosine)
        .expect("test: create");
    let docs = db.get_vector_collection("docs").expect("test: docs");
    (dir, db, docs)
}

fn sparse_point(id: u64, terms: &[(u32, f32)]) -> Point {
    let mut map = BTreeMap::new();
    map.insert(String::new(), SparseVector::new(terms.to_vec()));
    Point::with_sparse(id, vec![1.0, 0.0], None, Some(map))
}

fn ids_matching(docs: &VectorCollection, term: u32) -> Vec<u64> {
    docs.sparse_search(&SparseVector::new(vec![(term, 1.0)]), 10, "")
        .expect("test: sparse search")
        .into_iter()
        .map(|r| r.point.id)
        .collect()
}

#[test]
fn a_re_upserted_point_stops_matching_the_terms_it_dropped() {
    let (_dir, _db, docs) = docs();
    docs.upsert(vec![sparse_point(1, &[(7, 1.0)])])
        .expect("test: first upsert");
    docs.upsert(vec![sparse_point(1, &[(9, 1.0)])])
        .expect("test: second upsert");

    assert!(ids_matching(&docs, 7).is_empty(), "term 7 was dropped");
    assert_eq!(ids_matching(&docs, 9), vec![1]);
}

#[test]
fn a_re_upserted_point_survives_a_reopen_without_its_dropped_terms() {
    let (dir, db, docs) = docs();
    docs.upsert(vec![sparse_point(1, &[(7, 1.0)])])
        .expect("test: first upsert");
    docs.upsert(vec![sparse_point(1, &[(9, 1.0)])])
        .expect("test: second upsert");
    drop(docs);
    drop(db);

    let db = Database::open(dir.path()).expect("test: reopen");
    let docs = db.get_vector_collection("docs").expect("test: docs");
    assert!(ids_matching(&docs, 7).is_empty(), "term 7 was dropped");
    assert_eq!(ids_matching(&docs, 9), vec![1]);
}

#[test]
fn a_point_re_upserted_without_a_sparse_index_leaves_that_index() {
    let (_dir, _db, docs) = docs();
    let mut both = BTreeMap::new();
    both.insert(String::new(), SparseVector::new(vec![(7, 1.0)]));
    both.insert("title".to_string(), SparseVector::new(vec![(3, 1.0)]));
    docs.upsert(vec![Point::with_sparse(
        1,
        vec![1.0, 0.0],
        None,
        Some(both),
    )])
    .expect("test: first upsert");
    docs.upsert(vec![Point::new(1, vec![1.0, 0.0], None)])
        .expect("test: dense-only re-upsert");

    assert!(
        ids_matching(&docs, 7).is_empty(),
        "the default index let go"
    );
    let title = docs
        .sparse_search(&SparseVector::new(vec![(3, 1.0)]), 10, "title")
        .expect("test: title search");
    assert!(title.is_empty(), "the named index let go");
}

#[test]
fn the_last_duplicate_in_one_batch_wins() {
    let (_dir, _db, docs) = docs();
    docs.upsert(vec![
        sparse_point(1, &[(7, 1.0)]),
        sparse_point(1, &[(9, 1.0)]),
    ])
    .expect("test: upsert");
    assert!(
        ids_matching(&docs, 7).is_empty(),
        "the first duplicate lost"
    );
    assert_eq!(ids_matching(&docs, 9), vec![1]);
}

#[test]
fn a_non_finite_sparse_weight_is_refused_with_the_invalid_vector_code() {
    let (_dir, _db, docs) = docs();
    for weight in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let err = docs
            .upsert(vec![sparse_point(1, &[(7, weight)])])
            .expect_err("a non-finite weight must be refused");
        assert_eq!(err.code(), "VELES-005", "weight {weight}: {err}");
    }
    assert!(docs.is_empty(), "nothing was stored");
}

#[test]
fn a_non_finite_sparse_query_weight_is_refused() {
    let (_dir, _db, docs) = docs();
    docs.upsert(vec![sparse_point(1, &[(7, 1.0)])])
        .expect("test: upsert");
    let err = docs
        .sparse_search(&SparseVector::new(vec![(7, f32::NAN)]), 10, "")
        .expect_err("a NaN query weight must be refused");
    assert_eq!(err.code(), "VELES-005", "{err}");
}
