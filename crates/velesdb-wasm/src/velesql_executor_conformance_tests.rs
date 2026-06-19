//! Executor-level VelesQL conformance for the WASM executor.
//!
//! Companion to `velesdb-core/tests/velesql_executor_conformance.rs`. Loads the
//! SAME golden fixture (`conformance/velesql_executor_cases.json`), builds the
//! dataset **through the WASM executor** (`INSERT INTO ... VALUES`), runs each
//! case query through `velesql_exec::execute`, and asserts the result ids and
//! ordering match the golden `expect_ids` derived from `velesdb-core`.
//!
//! Native-target test (`#[test]`, not `wasm_bindgen_test`) that exercises
//! `DatabaseInner` through the executor dispatcher, mirroring
//! `velesql_exec_tests.rs`. The internal `DatabaseInner` / `execute` symbols
//! are `pub(crate)`, so this suite lives in-crate (like the other
//! `velesql_exec_*` test modules) rather than under `tests/`.
//!
//! # Case coverage
//!
//! All ten `cases` (X001–X010) are runnable by the WASM executor (scalar WHERE
//! filter, single- and multi-column ORDER BY, the ascending-id tie-break, and
//! bounded top-k) and are asserted against the golden ids. WASM runs its OWN
//! SELECT/ORDER BY pipeline (`velesql_select`/`velesql_orderby`), independent of
//! the core executor, so these goldens pin it against the `velesdb-core` result
//! set rather than assuming shared-executor equivalence.
//!
//! # known_bugs (B001)
//!
//! B001 is the core bug where `ORDER BY <column> + LIMIT` applies the LIMIT
//! before the sort on the bounded top-k path. The WASM SELECT pipeline
//! (`velesql_select::finalize_plain`) sorts the full row set *first* and only
//! then applies `skip(offset).take(limit)`, so it does NOT reproduce B001 — it
//! already returns the CORRECT golden result. The B001 case is therefore
//! asserted directly here as a non-bug for WASM (no `#[ignore]`).

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

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
    let content = std::fs::read_to_string(fixture_path()).expect("test: read executor fixture");
    serde_json::from_str(&content).expect("test: parse executor fixture")
}

/// Renders a payload scalar as a VelesQL literal (quoting strings).
fn payload_literal(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        other => other.to_string(),
    }
}

/// Builds the dataset *through the WASM executor* by issuing one
/// `INSERT INTO <coll> (id, vector, <payload keys...>) VALUES (...)` per point,
/// with the vector bound to the `$v` parameter (the canonical WASM vector-
/// insert contract — inline vector literals are unsupported).
fn build_dataset(ds: &Dataset) -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection(&ds.collection, ds.dimension, &ds.metric)
        .expect("test: create dataset collection");

    for point in &ds.points {
        let Value::Object(obj) = &point.payload else {
            panic!("test: fixture payload must be a JSON object");
        };
        let mut columns = vec!["id".to_string(), "vector".to_string()];
        let mut values = vec![point.id.to_string(), "$v".to_string()];
        for (key, val) in obj {
            columns.push(key.clone());
            values.push(payload_literal(val));
        }
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            ds.collection,
            columns.join(", "),
            values.join(", ")
        );
        let vector_json =
            serde_json::to_string(&point.vector).expect("test: serialize fixture vector");
        let params = format!("{{\"v\": {vector_json}}}");
        execute(&mut db, &sql, Some(&params))
            .unwrap_or_else(|e| panic!("test: seed point {} failed: {e}", point.id));
    }
    db
}

/// Runs every case through the WASM executor and asserts ids + ordering.
fn assert_cases(db: &mut DatabaseInner, cases: &[Case]) {
    for case in cases {
        let result = execute(db, &case.query, None)
            .unwrap_or_else(|e| panic!("case {} failed to execute: {e}", case.id));
        let ids: Vec<u64> = result
            .rows_ref()
            .iter()
            .map(crate::velesql_result::QueryResultRow::id)
            .collect();
        assert_eq!(
            ids, case.expect_ids,
            "WASM executor conformance mismatch for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// All golden `cases` (X001–X010) must reproduce the core result set exactly.
#[test]
fn test_wasm_velesql_executor_conformance_fixture_cases() {
    let fixture = load_fixture();
    let mut db = build_dataset(&fixture.dataset);
    assert_cases(&mut db, &fixture.cases);
}

/// B001 is a core-only bug: the WASM SELECT pipeline sorts before applying
/// LIMIT, so it returns the CORRECT golden result. Asserting it here locks in
/// that the WASM executor is NOT affected by the core bounded-top-k bug.
#[test]
fn test_wasm_velesql_executor_known_bugs_are_correct() {
    let fixture = load_fixture();
    let mut db = build_dataset(&fixture.dataset);
    assert_cases(&mut db, &fixture.known_bugs);
}
