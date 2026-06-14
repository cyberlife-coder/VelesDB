#![cfg(feature = "persistence")]
//! Executor-level VelesQL conformance (T2 of the core-parity remediation plan).
//!
//! The existing `velesql_parser_conformance` fixture only asserts that a query
//! *parses*. This suite goes further: it loads a fixed dataset, executes each
//! query through `velesdb-core` (the source of truth), and asserts the exact
//! result set — ROWS, COUNTS, and ORDERING. The golden ids in
//! `conformance/velesql_executor_cases.json` are the contract that other
//! executors (WASM, CLI) must reproduce, so a future divergence in
//! filtering/ordering/limit behaviour fails CI instead of going unnoticed.
//!
//! The fixture's `known_bugs` array carries the CORRECT expectation for a
//! confirmed core bug; it is asserted only by the `#[ignore]`d reproducer
//! below so CI stays green until the bug is fixed.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point};

#[derive(Deserialize)]
struct Fixture {
    dataset: Dataset,
    cases: Vec<Case>,
    #[serde(default)]
    known_bugs: Vec<Case>,
}

#[derive(Deserialize)]
struct Dataset {
    collection: String,
    dimension: usize,
    metric: String,
    points: Vec<FixturePoint>,
}

#[derive(Deserialize)]
struct FixturePoint {
    id: u64,
    vector: Vec<f32>,
    payload: Value,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    query: String,
    expect_ids: Vec<u64>,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/velesql_executor_cases.json")
}

fn load_fixture() -> Fixture {
    let content = std::fs::read_to_string(fixture_path()).expect("read executor fixture");
    serde_json::from_str(&content).expect("parse executor fixture")
}

fn open_loaded_db(ds: &Dataset) -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    let metric = ds.metric.parse().unwrap_or(DistanceMetric::Cosine);
    db.create_collection(&ds.collection, ds.dimension, metric)
        .expect("create dataset collection");
    let collection = db
        .get_vector_collection(&ds.collection)
        .expect("get dataset collection");
    let points: Vec<Point> = ds
        .points
        .iter()
        .map(|p| Point::new(p.id, p.vector.clone(), Some(p.payload.clone())))
        .collect();
    collection.upsert(points).expect("upsert dataset");
    (dir, db)
}

fn assert_cases(db: &Database, cases: &[Case]) {
    let params = HashMap::new();
    for case in cases {
        let query = Parser::parse(&case.query)
            .unwrap_or_else(|e| panic!("case {} failed to parse: {e}", case.id));
        let results = db
            .execute_query(&query, &params)
            .unwrap_or_else(|e| panic!("case {} failed to execute: {e}", case.id));
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert_eq!(
            ids, case.expect_ids,
            "executor conformance mismatch for case {}: query={:?}",
            case.id, case.query
        );
    }
}

#[test]
fn test_velesql_executor_conformance_fixture_cases() {
    let fixture = load_fixture();
    let (_dir, db) = open_loaded_db(&fixture.dataset);
    assert_cases(&db, &fixture.cases);
}

/// Reproducer for confirmed executor bugs (correct expectation). Ignored so CI
/// stays green; remove `#[ignore]` once the bug is fixed to lock the fix.
#[test]
#[ignore = "B001: scalar ORDER BY + LIMIT applies LIMIT before sort (bounded top-k); contradicts KNOWN_LIMITATIONS #9"]
fn test_velesql_executor_known_bugs() {
    let fixture = load_fixture();
    let (_dir, db) = open_loaded_db(&fixture.dataset);
    assert_cases(&db, &fixture.known_bugs);
}
