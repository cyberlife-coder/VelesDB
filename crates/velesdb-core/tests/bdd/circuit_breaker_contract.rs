//! BDD: a collection's circuit breaker counts the engine's failures, never
//! the caller's — driven through `Database::execute_query`.
//!
//! The breaker exists to shed load from a collection that keeps failing. A
//! failure the caller chose or caused (its own `WITH (timeout_ms)` budget, a
//! cardinality ceiling its query crossed) is answered the same way however
//! healthy the collection is, so it must not count: five of them in a row
//! used to refuse every query on the collection for the recovery window. A
//! timeout under the server's own limit is load the engine could not carry,
//! and still counts — wherever it fires, the MATCH traversal included.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point, QueryLimits};

use super::helpers::{create_test_db, execute_sql_with_params};

/// Failures the breaker tolerates before opening, in every scenario here.
const THRESHOLD: u32 = 2;

/// Enough requests to have opened the breaker several times over.
const REPEATS: usize = 5;

/// Rows in `docs`: enough that a full `ORDER BY` scan takes well over the
/// 1 ms budgets below in a test build.
const ROWS: u64 = 5_000;

/// Creates `docs` with [`ROWS`] `Doc` nodes and applies `limits` with the
/// breaker opening after [`THRESHOLD`] failures for a minute.
fn setup(db: &Database, limits: &QueryLimits) {
    db.create_vector_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create docs");
    let docs = db.get_vector_collection("docs").expect("test: get docs");
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (0..ROWS)
        .map(|i| {
            Point::new(
                i,
                vec![1.0, i as f32 / ROWS as f32, 0.5, 0.25],
                Some(json!({"_labels": ["Doc"], "rank": i})),
            )
        })
        .collect();
    docs.upsert(points).expect("test: upsert docs");
    let mut limits = limits.clone();
    limits.circuit_failure_threshold = THRESHOLD;
    limits.circuit_recovery_seconds = 60;
    db.update_guardrails(&limits);
}

fn breaker_is_closed(db: &Database) -> bool {
    db.get_vector_collection("docs")
        .expect("test: get docs")
        .guard_rails()
        .circuit_breaker
        .check()
        .is_ok()
}

fn run(db: &Database, sql: &str) -> velesdb_core::Result<usize> {
    let params = HashMap::from([("_collection".to_string(), json!("docs"))]);
    execute_sql_with_params(db, sql, &params).map(|rows| rows.len())
}

/// Asserts the query stopped on a guard rail, so each scenario proves the
/// phenomenon it is about happened before asserting anything about it.
fn assert_guard_rail_stop(result: velesdb_core::Result<usize>, what: &str) {
    let err = result.expect_err(what);
    assert_eq!(err.code(), "VELES-027", "{what}: {err}");
}

const FULL_SCAN: &str = "SELECT * FROM docs ORDER BY rank DESC LIMIT 5000";

#[test]
fn scenario_a_client_chosen_timeout_never_opens_the_breaker() {
    let (_dir, db) = create_test_db();
    setup(&db, &QueryLimits::default());

    for _ in 0..REPEATS {
        assert_guard_rail_stop(
            run(&db, &format!("{FULL_SCAN} WITH (timeout_ms = 1)")),
            "a full scan cannot finish in the 1 ms the client asked for",
        );
    }

    assert!(
        breaker_is_closed(&db),
        "the client's own budget is not the engine's failure"
    );
    assert_eq!(
        run(&db, FULL_SCAN).expect("test: the collection keeps serving"),
        5_000
    );
}

#[test]
fn scenario_a_cardinality_ceiling_never_opens_the_breaker() {
    let (_dir, db) = create_test_db();
    setup(
        &db,
        &QueryLimits {
            max_cardinality: 1,
            ..QueryLimits::default()
        },
    );

    for _ in 0..REPEATS {
        assert_guard_rail_stop(
            run(&db, "SELECT * FROM docs LIMIT 10"),
            "ten rows cross a ceiling of one",
        );
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
fn scenario_a_server_limit_timeout_still_opens_the_breaker() {
    let (_dir, db) = create_test_db();
    setup(
        &db,
        &QueryLimits {
            timeout_ms: 1,
            ..QueryLimits::default()
        },
    );

    for _ in 0..THRESHOLD {
        assert_guard_rail_stop(
            run(&db, FULL_SCAN),
            "a full scan cannot finish in the server's 1 ms limit",
        );
    }

    assert!(
        !breaker_is_closed(&db),
        "the engine's timeouts must open the breaker"
    );
}

/// A MATCH traversal stopped by the server's time limit fails inside the
/// traversal itself, before any of the result-finalizing steps run; that
/// failure is recorded like any other.
#[test]
fn scenario_a_match_traversal_failure_is_recorded() {
    let (_dir, db) = create_test_db();
    setup(
        &db,
        &QueryLimits {
            timeout_ms: 1,
            ..QueryLimits::default()
        },
    );
    let docs = db.get_vector_collection("docs").expect("test: get docs");
    for target in 1..ROWS {
        let edge = GraphEdge::new(target, 0, target, "LINKS").expect("test: edge");
        docs.add_edge(edge).expect("test: add edge");
    }

    for _ in 0..THRESHOLD {
        assert_guard_rail_stop(
            run(&db, "MATCH (a)-[:LINKS]->(b) RETURN b.rank LIMIT 100000"),
            "a traversal of 5 000 edges cannot finish in 1 ms",
        );
    }

    assert!(
        !breaker_is_closed(&db),
        "a traversal's engine failure must count"
    );
}
