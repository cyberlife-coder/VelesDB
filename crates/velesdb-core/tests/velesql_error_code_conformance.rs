#![cfg(all(test, feature = "persistence"))]
//! Error-CODE conformance for live `VelesQL` query-path rejections (backlog #12).
//!
//! Query-SHAPE and BIND-PARAM failures must classify as `Error::Query`
//! (`VELES-010`), not `Error::Config` (`VELES-009`). The TS SDK narrows the
//! engine `code` field into `QueryError`/`ConfigError`, so a query the caller
//! wrote wrong (unsupported shape, missing/malformed bind param) must surface
//! as a query error — not an engine-configuration error.
//!
//! These cases lock the contract so a future regression that re-tags a
//! query-path reject as `VELES-009` fails CI.

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

/// Two-dimensional "docs" collection with three rows for query execution.
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
    let points = vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({ "category": "tech" }))),
        Point::new(2, vec![0.0, 1.0], Some(json!({ "category": "news" }))),
        Point::new(3, vec![0.7, 0.7], Some(json!({ "category": "tech" }))),
    ];
    collection.upsert(points).expect("upsert");
    (collection, dir)
}

fn run_err(collection: &VectorCollection, query: &str) -> velesdb_core::Error {
    collection
        .execute_query_str(query, &HashMap::new())
        .expect_err("query expected to reject")
}

#[test]
fn unsupported_query_shape_multiple_similarity_in_or_is_velesq_query() {
    let (collection, _dir) = setup_docs();
    // Two similarity() predicates under OR is an unsupported query SHAPE.
    let err = run_err(
        &collection,
        "SELECT * FROM docs WHERE similarity(vector, $a) > 0.8 \
         OR similarity(vector, $b) > 0.7 LIMIT 10",
    );
    assert_eq!(
        err.code(),
        "VELES-010",
        "unsupported query shape must be VELES-010 (Query), got: {err}"
    );
}

#[test]
fn missing_bind_param_is_velesq_query() {
    let (collection, _dir) = setup_docs();
    // $missing is never provided in the params map.
    let err = run_err(
        &collection,
        "SELECT * FROM docs WHERE vector NEAR $missing LIMIT 5",
    );
    assert_eq!(
        err.code(),
        "VELES-010",
        "missing bind param must be VELES-010 (Query), got: {err}"
    );
}
