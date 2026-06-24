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
fn single_branch_fusion_reject_is_not_v006_similarity_message() {
    let (collection, _dir) = setup_docs();
    // A similarity()-only query carries no second retrieval branch, so the
    // trailing USING FUSION clause is a misconfiguration. The reject must NOT
    // borrow the misleading V006 "similarity() requires a vector search
    // context" code/message — it must be fusion-specific.
    let err = run_err(
        &collection,
        "SELECT * FROM docs WHERE similarity(vector, $q) > 0.5 \
         LIMIT 10 USING FUSION(strategy = 'maximum')",
    );
    let msg = err.to_string();
    assert!(
        !msg.contains("V006"),
        "fusion misconfig must not be tagged V006, got: {msg}"
    );
    assert!(
        !msg.contains("similarity() requires a vector search context"),
        "fusion misconfig must not reuse the similarity-context message, got: {msg}"
    );
    assert!(
        msg.contains("FUSION"),
        "fusion misconfig message must be fusion-specific, got: {msg}"
    );
}

#[test]
fn rsf_weight_sum_fusion_reject_is_not_v006_similarity_message() {
    let (collection, _dir) = setup_docs();
    // RSF dense_w + sparse_w != 1.0 over a real two-branch hybrid is a fusion
    // weight misconfiguration; it must classify honestly, not as V006.
    let err = run_err(
        &collection,
        "SELECT * FROM docs WHERE vector NEAR $q AND vector SPARSE_NEAR $sv \
         LIMIT 10 USING FUSION(strategy = 'rsf', dense_w = 0.7, sparse_w = 0.7)",
    );
    let msg = err.to_string();
    assert!(
        !msg.contains("V006"),
        "rsf weight-sum reject must not be tagged V006, got: {msg}"
    );
    assert!(
        !msg.contains("similarity() requires a vector search context"),
        "rsf weight-sum reject must not reuse the similarity-context message, got: {msg}"
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
