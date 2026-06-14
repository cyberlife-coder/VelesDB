#![cfg(all(test, feature = "persistence"))]
//! Regression: a scalar (non-similarity) `ORDER BY <col> ... LIMIT k` must sort
//! by the ORDER BY key BEFORE applying the LIMIT, so the bounded result equals
//! the unbounded path truncated to `k` (`KNOWN_LIMITATIONS` #9).
//!
//! Before the fix, the candidate fetch was capped at `k` in storage order, then
//! only that capped window was sorted — `... WHERE year >= 2019 ORDER BY year
//! DESC LIMIT 2` returned the first two matching rows by id [2, 1] instead of
//! the two newest [4, 2].

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

/// Builds the 5-row "docs" collection: ids 1..=5 with years 2020, 2022, 2021,
/// 2023, 2019 respectively (storage/id order deliberately ≠ year order).
fn setup_docs() -> (VectorCollection, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let collection = VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("create collection");

    let years = [(1u64, 2020), (2, 2022), (3, 2021), (4, 2023), (5, 2019)];
    let points: Vec<Point> = years
        .iter()
        .map(|&(id, year)| Point::new(id, vec![1.0, 0.0], Some(json!({ "year": year }))))
        .collect();
    // Years live in the payload; the vector is irrelevant to a scalar ORDER BY.
    collection.upsert(points).expect("upsert");
    (collection, dir)
}

fn ids(results: &[velesdb_core::point::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

#[test]
fn test_scalar_order_by_desc_limit_sorts_before_truncating() {
    let (collection, _dir) = setup_docs();
    let results = collection
        .execute_query_str(
            "SELECT * FROM docs WHERE year >= 2019 ORDER BY year DESC LIMIT 2",
            &HashMap::new(),
        )
        .expect("query");
    // Two newest: id 4 (2023), id 2 (2022).
    assert_eq!(ids(&results), vec![4, 2]);
}

#[test]
fn test_scalar_order_by_asc_limit_sorts_before_truncating() {
    let (collection, _dir) = setup_docs();
    let results = collection
        .execute_query_str(
            "SELECT * FROM docs WHERE year >= 2019 ORDER BY year ASC LIMIT 2",
            &HashMap::new(),
        )
        .expect("query");
    // Two oldest: id 5 (2019), id 1 (2020).
    assert_eq!(ids(&results), vec![5, 1]);
}

#[test]
fn test_scalar_order_by_no_limit_control_is_fully_sorted() {
    let (collection, _dir) = setup_docs();
    let results = collection
        .execute_query_str(
            "SELECT * FROM docs WHERE year >= 2019 ORDER BY year DESC",
            &HashMap::new(),
        )
        .expect("query");
    // Full descending order; LIMIT 2 must be a prefix of this control.
    assert_eq!(ids(&results), vec![4, 2, 3, 1, 5]);
}
