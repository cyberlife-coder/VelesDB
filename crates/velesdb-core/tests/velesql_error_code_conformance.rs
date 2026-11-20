#![cfg(all(test, feature = "persistence"))]
//! Error-CODE conformance for live `VelesQL` query-path rejections (backlog #12).
//!
//! Query-SHAPE failures must classify as `Error::Query` (`VELES-010`) and
//! BIND-PARAM failures as `Error::QueryParameter` (`VELES-038`), neither as
//! `Error::Config` (`VELES-009`). The TS SDK narrows the engine `code` field
//! into `QueryError`/`QueryParameterError`/`ConfigError`, so a query the
//! caller wrote wrong (unsupported shape) must surface as a query error, and a
//! missing or malformed bind param as a parameter error — never as an
//! engine-configuration error.
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
fn missing_bind_param_is_a_query_parameter_error() {
    let (collection, _dir) = setup_docs();
    // $missing is never provided in the params map.
    let err = run_err(
        &collection,
        "SELECT * FROM docs WHERE vector NEAR $missing LIMIT 5",
    );
    assert_eq!(
        err.code(),
        "VELES-038",
        "missing bind param must be VELES-038 (QueryParameter), got: {err}"
    );
}

/// A database with the "docs" collection of [`setup_docs`], for the statement
/// families only [`velesdb_core::Database::execute_query`] runs (DML, MATCH).
fn setup_db() -> (velesdb_core::Database, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = velesdb_core::Database::open(dir.path()).expect("open database");
    db.create_vector_collection("docs", 2, DistanceMetric::Cosine)
        .expect("create collection");
    db.get_vector_collection("docs")
        .expect("docs")
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({ "category": "tech" })),
        )])
        .expect("upsert");
    (db, dir)
}

/// Runs `sql` with `$v` bound to `v` (and "docs" as the `_collection` a
/// MATCH runs on) and returns the error it must fail with.
fn run_db_err(db: &velesdb_core::Database, sql: &str, v: serde_json::Value) -> velesdb_core::Error {
    let query = velesdb_core::velesql::Parser::parse(sql).expect("valid VelesQL");
    let params = HashMap::from([
        ("v".to_string(), v),
        ("_collection".to_string(), json!("docs")),
    ]);
    db.execute_query(&query, &params)
        .expect_err("statement expected to reject")
}

/// The values a vector `$v` cannot be bound to: not an array, and an array
/// holding an element that is no number.
fn mistyped_vectors() -> [serde_json::Value; 2] {
    [json!("x"), json!([1.0, "x"])]
}

/// Asserts `err` is the parameter error (`VELES-038`) naming `$v`.
fn assert_parameter_error_on_v(err: &velesdb_core::Error, context: &str) {
    assert!(
        matches!(err, velesdb_core::Error::QueryParameter { name, .. } if name == "v"),
        "{context}: expected a parameter error naming $v, got: {err}"
    );
    assert_eq!(err.code(), "VELES-038", "{context}: {err}");
}

/// An `INSERT` or `UPDATE` vector `$v` bound to a value that is no vector is
/// a parameter error, as a `NEAR $v` is: the DML path converted it itself
/// and refused it with a query error (`VELES-010`), so only an unbound `$v`
/// was a parameter error.
#[test]
fn dml_vector_parameter_of_the_wrong_type_is_a_query_parameter_error() {
    let (db, _dir) = setup_db();
    for statement in [
        "INSERT INTO docs (id, vector, category) VALUES (2, $v, 'news')",
        "UPSERT INTO docs (id, vector, category) VALUES (1, $v, 'news')",
        "UPDATE docs SET vector = $v WHERE id = 1",
    ] {
        for v in mistyped_vectors() {
            let err = run_db_err(&db, statement, v.clone());
            assert_parameter_error_on_v(&err, &format!("{statement} with $v = {v}"));
        }
    }
}

/// A MATCH `similarity()` vector `$v` holding an element that is no number
/// is a parameter error: MATCH used to drop the element and score the
/// shorter vector.
#[test]
fn match_vector_parameter_with_a_non_numeric_element_is_a_query_parameter_error() {
    let (db, _dir) = setup_db();
    let err = run_db_err(
        &db,
        "MATCH (d) WHERE similarity(d.vector, $v) > 0.5 RETURN d LIMIT 5",
        json!([1.0, "x"]),
    );
    assert_parameter_error_on_v(&err, "MATCH similarity()");
}

/// A `_collection` that is not a non-empty string is a parameter error
/// (`VELES-038`) naming the key, before the read gate or the executor runs:
/// an empty name read as the collection `''` (`VELES-002`), a non-string one
/// as no target at all (`VELES-010`).
#[test]
fn a_match_collection_param_that_names_no_collection_is_a_query_parameter_error() {
    let (db, _dir) = setup_db();
    let query =
        velesdb_core::velesql::Parser::parse("MATCH (n) RETURN n LIMIT 5").expect("valid VelesQL");
    for bad in [json!(""), json!(7)] {
        let params = HashMap::from([(velesdb_core::velesql::COLLECTION_PARAM.to_string(), bad)]);
        let err = db.execute_query(&query, &params).expect_err("refused");
        assert_eq!(err.code(), "VELES-038", "{err}");
        assert!(err.to_string().contains("$_collection"), "{err}");
    }
}
