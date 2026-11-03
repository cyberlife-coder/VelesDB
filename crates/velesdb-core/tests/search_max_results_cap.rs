#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss)] // indices into f32 coordinates, deliberately

//! `search.max_results` is a ceiling on the rows a single call hands back —
//! asserted on results, not on a getter (#2591).
//!
//! The knob was parsed and validated but ignored until the cap was wired:
//! every surface below used to return the full caller `k`/`LIMIT`. The tests
//! therefore assert the returned row count against a fixture that always has
//! more rows than the cap, so a missing clamp fails rather than vacuously
//! passing on an empty fixture.
//!
//! The cap lives at the caller-visible boundary (typed collection wrappers and
//! `Database`), not inside the engine: internal machinery — compound operands,
//! union legs, MATCH candidate pools — keeps its own fetch windows, and only
//! what a caller receives is bounded.

use std::collections::HashMap;
use velesdb_core::{velesql::Parser, Database, DistanceMetric, Point, VelesConfig};

const DIM: usize = 8;
const POINTS: u64 = 64;
const CAP: usize = 2;

/// Deterministic pseudo-random coordinates; the fixture must not change
/// between runs.
fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 10_000) as f32 / 10_000.0
        })
        .collect()
}

/// A `Database` opened with `search.max_results = CAP`, with `POINTS` points in
/// `docs` (each carrying `cat = id % 2` for the filtered path).
fn capped(dir: &tempfile::TempDir) -> (Database, velesdb_core::VectorCollection) {
    let mut config = VelesConfig::default();
    config.search.max_results = CAP;
    let db = Database::open_with_config(dir.path(), config).expect("test: open");
    db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let points: Vec<Point> = (0..POINTS)
        .map(|id| Point::new(id, vector(id), Some(serde_json::json!({ "cat": id % 2 }))))
        .collect();
    collection.upsert(points).expect("test: upsert");
    (db, collection)
}

/// The raw kNN surface (`/search`, SDK `search`).
#[test]
fn a_plain_search_is_capped() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (_db, collection) = capped(&dir);

    let results = collection
        .search(&vector(POINTS + 1), 10)
        .expect("test: search");
    assert_eq!(
        results.len(),
        CAP,
        "search.max_results={CAP} must cap a k=10 search; got {} rows",
        results.len()
    );
}

/// The post-filter kNN surface (`/search` with a filter body).
#[test]
fn a_filtered_search_is_capped() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (_db, collection) = capped(&dir);
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq(
        "cat",
        serde_json::json!(0),
    ));

    let results = collection
        .search_with_filter(&vector(POINTS + 1), 10, &filter)
        .expect("test: filtered search");
    assert_eq!(
        results.len(),
        CAP,
        "a filtered search returns at most search.max_results rows; got {}",
        results.len()
    );
}

/// The ids-only surface must not leak the uncapped k either.
#[test]
fn search_ids_is_capped() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (_db, collection) = capped(&dir);

    let results = collection
        .search_ids(&vector(POINTS + 1), 10)
        .expect("test: search_ids");
    assert_eq!(results.len(), CAP);
}

/// Each query in a batch is a query: the cap is per result set.
#[test]
fn a_batch_search_is_capped_per_query() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (_db, collection) = capped(&dir);
    let q1 = vector(POINTS + 1);
    let q2 = vector(POINTS + 2);

    let results = collection
        .search_batch_parallel(&[q1.as_slice(), q2.as_slice()], 10)
        .expect("test: batch search");
    assert_eq!(results.len(), 2);
    for (i, batch) in results.iter().enumerate() {
        assert_eq!(
            batch.len(),
            CAP,
            "batch result {i} exceeds the ceiling: {} rows",
            batch.len()
        );
    }
}

/// A `VelesQL` `SELECT` through `Database::execute_query` — the `/query` surface —
/// honors the ceiling on both an over-cap explicit LIMIT and the default limit.
#[test]
fn a_velesql_select_is_capped() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (db, _collection) = capped(&dir);
    let params = HashMap::new();

    let over_cap = Parser::parse("SELECT * FROM docs LIMIT 10").expect("test: parse");
    let results = db.execute_query(&over_cap, &params).expect("test: execute");
    assert_eq!(
        results.len(),
        CAP,
        "SELECT ... LIMIT 10 must return at most search.max_results; got {}",
        results.len()
    );

    let no_limit = Parser::parse("SELECT * FROM docs").expect("test: parse");
    let results = db.execute_query(&no_limit, &params).expect("test: execute");
    assert_eq!(
        results.len(),
        CAP,
        "a LIMIT-less SELECT is bounded by the ceiling; got {}",
        results.len()
    );
}

/// A `VelesQL` `MATCH` through `Database::execute_query` honors the ceiling.
#[test]
fn a_velesql_match_is_capped() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (db, _collection) = capped(&dir);
    let params = HashMap::from([("_collection".to_string(), serde_json::json!("docs"))]);

    let query = Parser::parse("MATCH (n) RETURN n").expect("test: parse");
    let results = db.execute_query(&query, &params).expect("test: execute");
    assert_eq!(
        results.len(),
        CAP,
        "MATCH without LIMIT returns the whole node set today; the ceiling bounds it; got {}",
        results.len()
    );
}

/// The direct `execute_match` surface (REST `/match`, the SDKs) honors the
/// ceiling, including an over-cap RETURN `LIMIT`.
#[test]
fn execute_match_is_capped() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (_db, collection) = capped(&dir);

    let query = Parser::parse("MATCH (n) RETURN n").expect("test: parse");
    let clause = query.match_clause.as_ref().expect("test: match clause");
    let results = collection
        .execute_match(clause, &HashMap::new())
        .expect("test: execute match");
    assert_eq!(results.len(), CAP);

    let query = Parser::parse("MATCH (n) RETURN n LIMIT 10").expect("test: parse");
    let clause = query.match_clause.as_ref().expect("test: match clause");
    let results = collection
        .execute_match(clause, &HashMap::new())
        .expect("test: execute match");
    assert_eq!(
        results.len(),
        CAP,
        "a RETURN LIMIT above the ceiling still returns at most max_results"
    );
}

/// Control: the ceiling is a ceiling, not a floor — a smaller caller `k` /
/// `LIMIT` still wins.
#[test]
fn a_smaller_request_is_not_padded_to_the_cap() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let (_db, collection) = capped(&dir);

    let results = collection
        .search(&vector(POINTS + 1), 1)
        .expect("test: search");
    assert_eq!(results.len(), 1);

    let query = Parser::parse("SELECT * FROM docs LIMIT 1").expect("test: parse");
    let results = collection
        .execute_query(&query, &HashMap::new())
        .expect("test: execute");
    assert_eq!(results.len(), 1);
}

/// Control: an unconfigured database keeps the historical unbounded behavior —
/// the default `max_results = 1000` is far above the `k` asked for here.
#[test]
fn a_default_config_keeps_full_k() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let db = Database::open_with_config(dir.path(), VelesConfig::default()).expect("test: open");
    db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let points: Vec<Point> = (0..POINTS)
        .map(|id| Point::new(id, vector(id), None))
        .collect();
    collection.upsert(points).expect("test: upsert");

    let results = collection
        .search(&vector(POINTS + 1), 10)
        .expect("test: search");
    assert_eq!(results.len(), 10);
}
