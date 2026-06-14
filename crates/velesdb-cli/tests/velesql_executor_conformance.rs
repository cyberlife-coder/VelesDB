#![allow(clippy::doc_markdown)]
//! Executor-level VelesQL conformance for the CLI executor.
//!
//! Companion to `velesdb-core/tests/velesql_executor_conformance.rs`. Loads the
//! SAME golden fixture (`conformance/velesql_executor_cases.json`), builds the
//! dataset **through the real `velesdb` binary** (`collection create` +
//! `data upsert`), runs each case query through the binary
//! (`query execute ... --format json`), and asserts the result ids and ordering
//! match the golden `expect_ids` derived from `velesdb-core`.
//!
//! The CLI has no library target, so — like `velesql_parser_conformance.rs` —
//! this integration test cannot call CLI internals directly. It instead drives
//! the compiled binary end-to-end with `assert_cmd`, which exercises the true
//! CLI execution path (`repl::execute_query` → `Database::execute_query` →
//! JSON output via `repl::print_result`).
//!
//! # Case coverage
//!
//! All five `cases` (X001–X005) are runnable by the CLI executor and asserted
//! against the golden ids.
//!
//! # known_bugs (B001)
//!
//! The CLI delegates SELECT execution to `velesdb-core::Database::execute_query`,
//! so it reproduces the core B001 bug verbatim (`ORDER BY <column> + LIMIT`
//! applies the LIMIT before the sort on the bounded top-k path — observed
//! `[2, 1]` instead of the correct `[4, 2]`). The B001 reproducer is therefore
//! `#[ignore]`d here, mirroring `test_velesql_executor_known_bugs` in core; the
//! fixture's `expect_ids` carries the CORRECT result so the test can be
//! un-ignored to lock the fix once core is patched.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde::Deserialize;
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Deserialize)]
struct Fixture {
    dataset: Dataset,
    cases: Vec<Case>,
    #[serde(default)]
    known_bugs: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Dataset {
    collection: String,
    dimension: usize,
    metric: String,
    points: Vec<FixturePoint>,
}

#[derive(Debug, Deserialize)]
struct FixturePoint {
    id: u64,
    vector: Vec<f32>,
    payload: Value,
}

#[derive(Debug, Deserialize)]
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

fn velesdb_cmd() -> Command {
    Command::cargo_bin("velesdb").expect("locate velesdb binary")
}

/// Builds the dataset on disk through the real CLI binary: one
/// `collection create` then one `data upsert` per fixture point.
fn build_dataset(db_path: &Path, ds: &Dataset) {
    velesdb_cmd()
        .args([
            "collection",
            "create",
            &db_path.to_string_lossy(),
            &ds.collection,
            "--dimension",
            &ds.dimension.to_string(),
            "--metric",
            &ds.metric,
        ])
        .assert()
        .success();

    for point in &ds.points {
        let vector_json = serde_json::to_string(&point.vector).expect("serialize fixture vector");
        let payload_json =
            serde_json::to_string(&point.payload).expect("serialize fixture payload");
        velesdb_cmd()
            .args([
                "data",
                "upsert",
                &db_path.to_string_lossy(),
                &ds.collection,
                "--id",
                &point.id.to_string(),
                "--vector",
                &vector_json,
                "--payload",
                &payload_json,
            ])
            .assert()
            .success();
    }
}

/// Runs a case query through the CLI and returns the result ids in order.
fn run_case_ids(db_path: &Path, query: &str) -> Vec<u64> {
    let output = velesdb_cmd()
        .args([
            "query",
            "execute",
            &db_path.to_string_lossy(),
            query,
            "--format",
            "json",
        ])
        .output()
        .expect("run query");
    assert!(
        output.status.success(),
        "query failed: {query}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let rows: Vec<Value> = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("parse JSON rows for query {query:?}: {e}\nstdout: {stdout}"));
    rows.iter()
        .map(|row| {
            row.get("id")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| panic!("row missing numeric id: {row}"))
        })
        .collect()
}

fn assert_cases(db_path: &Path, cases: &[Case]) {
    for case in cases {
        let ids = run_case_ids(db_path, &case.query);
        assert_eq!(
            ids, case.expect_ids,
            "CLI executor conformance mismatch for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// All golden `cases` (X001–X005) must reproduce the core result set exactly
/// when run through the CLI binary.
#[test]
fn test_cli_velesql_executor_conformance_fixture_cases() {
    let fixture = load_fixture();
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("conf_db");
    build_dataset(&db_path, &fixture.dataset);
    assert_cases(&db_path, &fixture.cases);
}

/// Reproducer for the core B001 bug as seen through the CLI (which delegates to
/// `Database::execute_query`). Ignored so CI stays green; un-ignore once the
/// core bounded-top-k LIMIT-before-sort bug is fixed.
#[test]
#[ignore = "B001: CLI delegates to core; scalar ORDER BY + LIMIT applies LIMIT before sort (bounded top-k)"]
fn test_cli_velesql_executor_known_bugs() {
    let fixture = load_fixture();
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("conf_db");
    build_dataset(&db_path, &fixture.dataset);
    assert_cases(&db_path, &fixture.known_bugs);
}
