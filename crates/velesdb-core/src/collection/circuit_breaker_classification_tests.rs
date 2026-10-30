//! The circuit breaker counts only failures that say the engine is unhealthy.
//!
//! A caller's mistake — an unsupported `ORDER BY`, a missing bind parameter, a
//! vector of the wrong dimension — says nothing about the collection's health.
//! Counted, five of them in a row opened the breaker and refused every query
//! on the collection for the recovery window: a client typo took the
//! collection down. These scenarios drive two execution paths that record
//! breaker outcomes (MATCH ordering, SELECT vector dispatch) with caller errors, then with a genuine overload, and
//! check the breaker against each.

use crate::collection::types::Collection;
use crate::guardrails::{CircuitState, GuardRails, QueryLimits};
use crate::point::Point;
use crate::velesql::Parser;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Failures the breaker tolerates before opening, in every scenario below.
const THRESHOLD: u32 = 2;

/// Enough caller errors to have opened the breaker several times over.
const CALLER_ERRORS: usize = 5;

/// A 4-dim collection of ten `Doc` nodes, whose breaker opens after
/// [`THRESHOLD`] failures and whose cardinality ceiling is `max_cardinality`.
fn collection_with_breaker(dir: &TempDir, max_cardinality: usize) -> Collection {
    let mut col = Collection::create(
        dir.path().join("docs"),
        4,
        crate::distance::DistanceMetric::Cosine,
    )
    .expect("test: create collection");
    let points: Vec<Point> = (0u64..10)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let x = i as f32 / 10.0;
            Point::new(
                i,
                vec![x, 0.1, 0.1, 0.1],
                Some(serde_json::json!({ "_labels": ["Doc"], "year": 2000 + i })),
            )
        })
        .collect();
    col.upsert(points).expect("test: upsert");
    col.runtime.guard_rails = Arc::new(GuardRails::with_limits(QueryLimits {
        max_cardinality,
        circuit_failure_threshold: THRESHOLD,
        circuit_recovery_seconds: 60,
        ..QueryLimits::default()
    }));
    col
}

fn breaker_state(col: &Collection) -> CircuitState {
    col.runtime.guard_rails.circuit_breaker.state()
}

/// A MATCH whose `ORDER BY` the executor cannot run fails with VELES-018 on
/// the ordered-MATCH surface (which takes a `MatchClause` and so never meets
/// the validator) — and leaves the breaker closed however often it is sent.
#[test]
fn unsupported_match_order_by_never_opens_the_breaker() {
    let dir = TempDir::new().expect("test: tempdir");
    let col = collection_with_breaker(&dir, 100_000);
    let query =
        Parser::parse("MATCH (d:Doc) RETURN d.year ORDER BY d LIMIT 3").expect("test: parse");
    let clause = query.match_clause.expect("test: MATCH clause");

    for _ in 0..CALLER_ERRORS {
        let err = col
            .match_query_ordered(&clause, &HashMap::new())
            .expect_err("test: ORDER BY a bare alias is unsupported");
        assert_eq!(err.code(), "VELES-018", "{err}");
    }

    assert_eq!(breaker_state(&col), CircuitState::Closed);
    let ok =
        Parser::parse("MATCH (d:Doc) RETURN d.year ORDER BY d.year LIMIT 3").expect("test: parse");
    col.match_query_ordered(&ok.match_clause.expect("test: clause"), &HashMap::new())
        .expect("test: the collection must keep serving queries");
}

/// A query vector of the wrong dimension fails with VELES-004 inside the
/// SELECT vector dispatch, another recording site, and must not count either.
#[test]
fn dimension_mismatch_never_opens_the_breaker() {
    let dir = TempDir::new().expect("test: tempdir");
    let col = collection_with_breaker(&dir, 100_000);
    let query =
        Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 3").expect("test: parse");
    let params = HashMap::from([("v".to_string(), serde_json::json!([1.0, 0.0]))]);

    for _ in 0..CALLER_ERRORS {
        let err = col
            .execute_query(&query, &params)
            .expect_err("test: a 2-dim vector against a 4-dim collection");
        assert_eq!(err.code(), "VELES-004", "{err}");
    }

    assert_eq!(breaker_state(&col), CircuitState::Closed);
}

/// Positive control: a guard-rail stop (here, the cardinality ceiling a
/// runaway MATCH crosses) is load on the engine and still opens the breaker
/// after [`THRESHOLD`] failures, on the same MATCH surface as above.
#[test]
fn guard_rail_stops_still_open_the_breaker() {
    let dir = TempDir::new().expect("test: tempdir");
    let col = collection_with_breaker(&dir, 1);
    let query =
        Parser::parse("MATCH (d:Doc) RETURN d.year ORDER BY d.year LIMIT 10").expect("test: parse");
    let clause = query.match_clause.expect("test: MATCH clause");

    for _ in 0..THRESHOLD {
        let err = col
            .match_query_ordered(&clause, &HashMap::new())
            .expect_err("test: ten rows against a ceiling of one");
        assert_eq!(err.code(), "VELES-027", "{err}");
        assert!(!err.is_caller_error(), "{err}");
    }

    assert_eq!(breaker_state(&col), CircuitState::Open);
}
