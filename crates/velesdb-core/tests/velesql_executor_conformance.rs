#![cfg(feature = "persistence")]
//! Executor-level `VelesQL` conformance (T2 of the core-parity remediation plan).
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
use velesdb_core::collection::graph::GraphEdge;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point};

#[derive(Deserialize)]
struct Fixture {
    dataset: Dataset,
    cases: Vec<Case>,
    #[serde(default)]
    known_bugs: Vec<Case>,
    #[serde(default)]
    extra_collections: Vec<Dataset>,
    #[serde(default)]
    join_cases: Vec<Case>,
    #[serde(default)]
    aggregate_cases: Vec<AggregateCase>,
    #[serde(default)]
    setops_cases: Vec<Case>,
    #[serde(default)]
    match_cases: Vec<MatchCase>,
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

/// A GROUP BY / HAVING golden case, asserted via [`Database::execute_aggregate`]
/// rather than [`Database::execute_query`] (issue #1544): `execute_query` only
/// applies GROUP BY when combined with a vector `NEAR` search (diversity
/// grouping); plain SQL aggregation is a separate entry point.
#[derive(Deserialize)]
struct AggregateCase {
    id: String,
    query: String,
    expect_groups: Vec<Value>,
}

/// A MATCH golden case (issue #1544). Core and WASM have genuinely different
/// graph data models (see `documented_divergences` D004 in the fixture), so
/// each engine gets its own query text over an equivalent graph topology; only
/// `expect_ids` is shared.
#[derive(Deserialize)]
struct MatchCase {
    id: String,
    core_query: String,
    expect_ids: Vec<u64>,
}

/// A graph edge used to build the `people` MATCH dataset (issue #1544). Not
/// part of the JSON fixture (which stays engine-agnostic for MATCH data) —
/// defined here alongside the fixed topology shared with the WASM harness.
struct FixtureEdge {
    id: u64,
    src: u64,
    dst: u64,
    label: &'static str,
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

/// Regression lock for B001 (scalar `ORDER BY` + `LIMIT`): formerly LIMIT was
/// applied before the sort; fixed in the executor so the bounded result now
/// equals the unbounded path truncated to k (`KNOWN_LIMITATIONS` #9).
#[test]
fn test_velesql_executor_known_bugs() {
    let fixture = load_fixture();
    let (_dir, db) = open_loaded_db(&fixture.dataset);
    assert_cases(&db, &fixture.known_bugs);
}

/// Loads the primary `dataset` plus every `extra_collections` entry into one
/// `Database`, for JOIN cases that span more than one collection (issue #1544).
fn open_loaded_db_multi(ds: &Dataset, extra: &[Dataset]) -> (TempDir, Database) {
    let (dir, db) = open_loaded_db(ds);
    for extra_ds in extra {
        let metric = extra_ds.metric.parse().unwrap_or(DistanceMetric::Cosine);
        db.create_collection(&extra_ds.collection, extra_ds.dimension, metric)
            .unwrap_or_else(|e| panic!("create extra collection {}: {e}", extra_ds.collection));
        let collection = db
            .get_vector_collection(&extra_ds.collection)
            .unwrap_or_else(|| panic!("get extra collection {}", extra_ds.collection));
        let points: Vec<Point> = extra_ds
            .points
            .iter()
            .map(|p| Point::new(p.id, p.vector.clone(), Some(p.payload.clone())))
            .collect();
        collection
            .upsert(points)
            .unwrap_or_else(|e| panic!("upsert extra collection {}: {e}", extra_ds.collection));
    }
    (dir, db)
}

/// JOIN (ON and USING) conformance against `velesdb-core` (issue #1544).
///
/// Uses the primary `dataset` (`docs`) plus every `extra_collections` entry
/// (`customers`/`orders` for ON, `customers_u`/`orders_u` for USING) so the
/// JOIN target collections exist alongside the base dataset. Reuses
/// `assert_cases`/`Database::execute_query` exactly like the plain SELECT
/// cases — JOIN resolution happens transparently inside `execute_query`.
#[test]
fn test_velesql_executor_conformance_join_cases() {
    let fixture = load_fixture();
    let (_dir, db) = open_loaded_db_multi(&fixture.dataset, &fixture.extra_collections);
    assert_cases(&db, &fixture.join_cases);
}

/// GROUP BY / HAVING conformance against `velesdb-core` (issue #1544).
///
/// Routed through [`Database::execute_aggregate`] (not `execute_query` — see
/// [`AggregateCase`]'s doc comment for why), and compared as JSON values
/// rather than point ids since a GROUP BY row is an aggregated group, not a
/// single point. Every case's query carries an explicit `ORDER BY` on the
/// group-by column(s) so the result order is deterministic (grouping itself
/// iterates a `HashMap` with no inherent order).
#[test]
fn test_velesql_executor_conformance_aggregate_cases() {
    let fixture = load_fixture();
    let (_dir, db) = open_loaded_db(&fixture.dataset);
    let params = HashMap::new();
    for case in &fixture.aggregate_cases {
        let query = Parser::parse(&case.query)
            .unwrap_or_else(|e| panic!("aggregate case {} failed to parse: {e}", case.id));
        let result = db
            .execute_aggregate(&query, &params)
            .unwrap_or_else(|e| panic!("aggregate case {} failed to execute: {e}", case.id));
        let groups = result
            .as_array()
            .unwrap_or_else(|| panic!("aggregate case {} did not return an array", case.id));
        assert_eq!(
            groups, &case.expect_groups,
            "aggregate conformance mismatch for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// UNION / INTERSECT / EXCEPT conformance against `velesdb-core` (issue #1544).
///
/// Compared as a **sorted set of ids**, not an ordered list: `apply_set_operation`
/// (`set_operations.rs`) always re-sorts the merged result by score descending;
/// when every row scores 0.0 (no vector `NEAR` in either branch) the tie-break
/// is `Vec::sort_unstable_by`'s implementation-defined order, which was
/// observed to differ between two consecutive runs of the same binary for the
/// identical query. Only membership and count are part of the golden contract
/// here (`documented_divergences` D003 in the fixture).
#[test]
fn test_velesql_executor_conformance_setops_cases() {
    let fixture = load_fixture();
    let (_dir, db) = open_loaded_db(&fixture.dataset);
    let params = HashMap::new();
    for case in &fixture.setops_cases {
        let query = Parser::parse(&case.query)
            .unwrap_or_else(|e| panic!("setops case {} failed to parse: {e}", case.id));
        let results = db
            .execute_query(&query, &params)
            .unwrap_or_else(|e| panic!("setops case {} failed to execute: {e}", case.id));
        let mut ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        ids.sort_unstable();
        let mut expected = case.expect_ids.clone();
        expected.sort_unstable();
        assert_eq!(
            ids, expected,
            "setops conformance mismatch (compared as a set) for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// Fixed graph topology shared by the core and WASM MATCH conformance tests
/// (issue #1544): a 4-node chain `1 -[:KNOWS]-> 2 -[:KNOWS]-> 3 -[:KNOWS]-> 4`.
/// Node ids and edges are identical on both sides; only the setup mechanism
/// and query text differ (see `documented_divergences` D004 in the fixture).
fn match_chain_edges() -> Vec<FixtureEdge> {
    vec![
        FixtureEdge {
            id: 100,
            src: 1,
            dst: 2,
            label: "KNOWS",
        },
        FixtureEdge {
            id: 101,
            src: 2,
            dst: 3,
            label: "KNOWS",
        },
        FixtureEdge {
            id: 102,
            src: 3,
            dst: 4,
            label: "KNOWS",
        },
    ]
}

/// MATCH (1- and 2-hop) conformance against `velesdb-core` (issue #1544).
///
/// Core treats vector-collection points as graph nodes: builds a `people`
/// collection with a `_labels` payload field, wires up the chain topology via
/// `Collection::add_edge`, and runs each case's `core_query`
/// (`SELECT ... FROM people WHERE MATCH (...)`) through the same
/// `Database::execute_query` path as every other SELECT case.
#[test]
fn test_velesql_executor_conformance_match_cases() {
    let fixture = load_fixture();
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("people", 4, DistanceMetric::Cosine)
        .expect("create people collection");
    let people = db
        .get_vector_collection("people")
        .expect("get people collection");
    let names = ["Ann", "Bo", "Cy", "Di"];
    let points: Vec<Point> = names
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let id = idx as u64 + 1;
            let mut vector = vec![0.0; 4];
            vector[idx] = 1.0;
            Point::new(
                id,
                vector,
                Some(serde_json::json!({ "_labels": ["Person"], "name": name })),
            )
        })
        .collect();
    people.upsert(points).expect("upsert people");
    for edge in match_chain_edges() {
        people
            .add_edge(GraphEdge::new(edge.id, edge.src, edge.dst, edge.label).expect("build edge"))
            .unwrap_or_else(|e| panic!("add edge {}: {e}", edge.id));
    }

    let params = HashMap::new();
    for case in &fixture.match_cases {
        let query = Parser::parse(&case.core_query)
            .unwrap_or_else(|e| panic!("match case {} failed to parse: {e}", case.id));
        let results = db
            .execute_query(&query, &params)
            .unwrap_or_else(|e| panic!("match case {} failed to execute: {e}", case.id));
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert_eq!(
            ids, case.expect_ids,
            "match conformance mismatch for case {}: core_query={:?}",
            case.id, case.core_query
        );
    }
}
