//! A collection's circuit breaker counts the engine's failures, never the
//! caller's — driven through `Database::execute_query`.
//!
//! The breaker exists to shed load from a collection that keeps failing. A
//! failure the caller chose or caused (its own `WITH (timeout_ms)` budget, a
//! cardinality ceiling its query crossed) is answered the same way however
//! healthy the collection is, so it must not count: five of them in a row
//! used to refuse every query on the collection for the recovery window. A
//! timeout under the server's own limit is load the engine could not carry,
//! and still counts — wherever it fires, the MATCH traversal included.
//!
//! Every timeout here is deterministic: a `QueryClockSkewGuard` moves every
//! query clock past any budget, so no scenario races the wall clock.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;

use crate::error::{GuardRailKind, TimeoutBudget};
use crate::guardrails::{QueryClockSkewGuard, QueryLimits};
use crate::velesql::Parser;
use crate::{Database, DistanceMetric, Error, GraphEdge, Point};

/// Failures the breaker tolerates before opening, in every scenario here.
const THRESHOLD: u32 = 2;

/// Enough requests to have opened the breaker several times over.
const REPEATS: usize = 5;

/// The server's time limit in every timeout scenario.
const LIMIT_MS: u64 = 1_000;

/// Far past every budget used here.
const SKEW_MS: u64 = 10_000_000;

/// Nodes in `docs`. The MATCH traversal checks its clock at every 100th
/// traversed edge, so the `0 -> 1..NODES` star must give it more than 100.
const NODES: u64 = 150;

/// Opens a database whose `docs` collection holds [`NODES`] `Doc` nodes,
/// linked `0 -> 1..NODES` by `LINKS` edges, under `limits` with the breaker
/// opening after [`THRESHOLD`] failures for a minute.
fn setup(limits: &QueryLimits) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_vector_collection("docs", 4, DistanceMetric::Cosine)
        .expect("test: create docs");
    let docs = db.get_vector_collection("docs").expect("test: get docs");
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (0..NODES)
        .map(|i| {
            Point::new(
                i,
                vec![1.0, i as f32 / NODES as f32, 0.5, 0.25],
                Some(json!({"_labels": ["Doc"], "rank": i})),
            )
        })
        .collect();
    docs.upsert(points).expect("test: upsert docs");
    for target in 1..NODES {
        let edge = GraphEdge::new(target, 0, target, "LINKS").expect("test: edge");
        docs.add_edge(edge).expect("test: add edge");
    }
    let mut limits = limits.clone();
    limits.circuit_failure_threshold = THRESHOLD;
    limits.circuit_recovery_seconds = 60;
    db.update_guardrails(&limits);
    (dir, db)
}

fn breaker_is_closed(db: &Database) -> bool {
    db.get_vector_collection("docs")
        .expect("test: get docs")
        .guard_rails()
        .circuit_breaker
        .check()
        .is_ok()
}

fn run(db: &Database, sql: &str) -> crate::Result<usize> {
    let query = Parser::parse(sql).map_err(|e| Error::Query(e.to_string()))?;
    let params = HashMap::from([("_collection".to_string(), json!("docs"))]);
    db.execute_query(&query, &params).map(|rows| rows.len())
}

/// Asserts the query stopped on the guard rail `kind`, so each scenario
/// proves the phenomenon it is about happened before asserting anything
/// about the breaker.
fn assert_stopped_by(result: crate::Result<usize>, kind: GuardRailKind) {
    match result {
        Err(Error::GuardRail { kind: got, .. }) => assert_eq!(got, kind),
        other => panic!("expected a {kind:?} guard-rail stop, got {other:?}"),
    }
}

fn server_limit() -> QueryLimits {
    QueryLimits::default().with_timeout_ms(LIMIT_MS)
}

const SCAN: &str = "SELECT * FROM docs LIMIT 10";

#[test]
fn a_client_chosen_timeout_never_opens_the_breaker() {
    let (_dir, db) = setup(&server_limit());
    {
        let _skew = QueryClockSkewGuard::activate(SKEW_MS);
        for _ in 0..REPEATS {
            assert_stopped_by(
                run(&db, &format!("{SCAN} WITH (timeout_ms = 100)")),
                GuardRailKind::Timeout(TimeoutBudget::Client),
            );
        }
    }

    assert!(
        breaker_is_closed(&db),
        "the client's own budget is not the engine's failure"
    );
    assert_eq!(
        run(&db, SCAN).expect("test: the collection keeps serving"),
        10
    );
}

/// A client budget of `0`, or one at or above the server's limit, leaves the
/// server's limit in force: a timeout under it is the engine's, and counts.
#[test]
fn a_client_budget_that_is_not_tighter_than_the_limit_is_the_servers() {
    for with in [
        "WITH (timeout_ms = 0)",
        "WITH (timeout_ms = 1000)",
        "WITH (timeout_ms = 90000)",
    ] {
        let (_dir, db) = setup(&server_limit());
        let _skew = QueryClockSkewGuard::activate(SKEW_MS);
        for _ in 0..THRESHOLD {
            assert_stopped_by(
                run(&db, &format!("{SCAN} {with}")),
                GuardRailKind::Timeout(TimeoutBudget::Configured),
            );
        }
        assert!(
            !breaker_is_closed(&db),
            "{with}: the server's limit ran out"
        );
    }
}

#[test]
fn a_cardinality_ceiling_never_opens_the_breaker() {
    let (_dir, db) = setup(&QueryLimits {
        max_cardinality: 1,
        ..QueryLimits::default()
    });

    for _ in 0..REPEATS {
        assert_stopped_by(run(&db, SCAN), GuardRailKind::Cardinality);
    }

    assert!(breaker_is_closed(&db), "an oversized query is the caller's");
    assert_eq!(
        run(&db, "SELECT * FROM docs LIMIT 1").expect("test: serving"),
        1
    );
}

/// Positive control: a timeout under the server's own limit is load the
/// engine could not carry, and opens the breaker after [`THRESHOLD`] of them.
#[test]
fn a_server_limit_timeout_opens_the_breaker() {
    let (_dir, db) = setup(&server_limit());
    let _skew = QueryClockSkewGuard::activate(SKEW_MS);

    for _ in 0..THRESHOLD {
        assert_stopped_by(
            run(&db, SCAN),
            GuardRailKind::Timeout(TimeoutBudget::Configured),
        );
    }

    assert!(
        !breaker_is_closed(&db),
        "the engine's timeouts must open the breaker"
    );
}

/// The traversal checks its clock at the 100th of the 149 edges it walks,
/// so under a skewed clock the MATCH fails inside the traversal, before the
/// result-finalizing steps that also record outcomes run. The breaker
/// opening is the traversal's own record site at work: a mutation run with
/// that site removed leaves the breaker closed and fails here.
#[test]
fn a_match_traversal_failure_is_recorded() {
    let (_dir, db) = setup(&server_limit());
    let _skew = QueryClockSkewGuard::activate(SKEW_MS);

    for _ in 0..THRESHOLD {
        assert_stopped_by(
            run(&db, "MATCH (a)-[:LINKS]->(b) RETURN b.rank LIMIT 1000"),
            GuardRailKind::Timeout(TimeoutBudget::Configured),
        );
    }

    assert!(
        !breaker_is_closed(&db),
        "a traversal's engine failure must count"
    );
}
