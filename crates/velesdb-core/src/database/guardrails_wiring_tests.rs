//! `Database::update_guardrails` reaches every limit it advertises.
//!
//! The update used to swap only the per-collection `QueryLimits` snapshot of
//! the collections already registered: the rate limiter and the circuit
//! breaker kept the values they were built with, and a collection created or
//! opened afterwards started from `QueryLimits::default()`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tempfile::tempdir;

use crate::guardrails::{GuardRailViolation, QueryLimits};
use crate::{Database, DistanceMetric};

fn limits_with_rate_and_breaker(qps: u32, threshold: u32) -> QueryLimits {
    QueryLimits {
        rate_limit_qps: qps,
        circuit_failure_threshold: threshold,
        ..QueryLimits::default()
    }
}

#[test]
fn update_reaches_the_rate_limiter_of_a_registered_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .unwrap();

    db.update_guardrails(&limits_with_rate_and_breaker(1, 5));

    let guards = db
        .get_vector_collection("docs")
        .unwrap()
        .guard_rails()
        .clone();
    guards
        .pre_check("client")
        .expect("first query within 1 qps");
    match guards.pre_check("client") {
        Err(GuardRailViolation::RateLimitExceeded { limit_qps }) => assert_eq!(limit_qps, 1),
        other => panic!("expected the updated 1 qps limit, got {other:?}"),
    }
}

#[test]
fn update_reaches_the_circuit_breaker_of_a_registered_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .unwrap();

    db.update_guardrails(&QueryLimits {
        circuit_failure_threshold: 1,
        circuit_recovery_seconds: 9_999,
        ..QueryLimits::default()
    });

    let guards = db
        .get_vector_collection("docs")
        .unwrap()
        .guard_rails()
        .clone();
    guards.circuit_breaker.record_failure();
    assert!(
        matches!(
            guards.pre_check("client"),
            Err(GuardRailViolation::CircuitOpen { .. })
        ),
        "one failure must trip a breaker reconfigured to threshold 1"
    );
}

#[test]
fn a_collection_created_after_the_update_starts_from_it() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.update_guardrails(&QueryLimits::default().with_max_depth(2));

    db.create_collection("vectors", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_metadata_collection("meta").unwrap();

    let vectors = db.get_vector_collection("vectors").unwrap();
    assert_eq!(vectors.guard_rails().limits().max_depth, 2);
    let meta = db.get_metadata_collection("meta").unwrap();
    assert_eq!(meta.inner.guard_rails().limits().max_depth, 2);
}

#[test]
fn a_collection_loaded_after_the_update_starts_from_it() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        db.create_collection("docs", 4, DistanceMetric::Cosine)
            .unwrap();
    }
    let db = Database::open(dir.path()).unwrap();
    db.update_guardrails(&limits_with_rate_and_breaker(7, 5));
    // Dropping the registry entry forces the disk-open fallback, the path a
    // collection takes when it is first reached after the update.
    db.vector_colls.write().clear();

    let reopened = db.get_vector_collection("docs").unwrap();
    assert_eq!(reopened.guard_rails().limits().rate_limit_qps, 7);
}

#[test]
fn guardrail_limits_reads_back_the_last_update() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(
        db.guardrail_limits().timeout_ms,
        QueryLimits::default().timeout_ms
    );

    db.update_guardrails(&QueryLimits::default().with_timeout_ms(1_234));
    assert_eq!(db.guardrail_limits().timeout_ms, 1_234);
}

#[test]
fn update_guardrails_with_merges_into_the_live_limits() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .unwrap();
    db.update_guardrails(&QueryLimits::default().with_max_depth(4));

    let applied = db
        .update_guardrails_with(|limits| limits.timeout_ms = 9_000)
        .unwrap();

    assert_eq!(
        applied.max_depth, 4,
        "an untouched field keeps its live value"
    );
    assert_eq!(applied.timeout_ms, 9_000);
    let coll = db.get_vector_collection("docs").unwrap();
    assert_eq!(coll.guard_rails().limits().max_depth, 4);
    assert_eq!(coll.guard_rails().limits().timeout_ms, 9_000);
}

#[test]
fn update_guardrails_with_refuses_limits_that_would_stop_every_query() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .unwrap();
    let before = db.guardrail_limits();

    let zero_qps = db.update_guardrails_with(|limits| {
        limits.timeout_ms = 9_000;
        limits.rate_limit_qps = 0;
    });
    let zero_threshold = db.update_guardrails_with(|limits| limits.circuit_failure_threshold = 0);

    assert!(matches!(zero_qps, Err(crate::Error::Config(ref m)) if m.contains("rate_limit_qps")));
    assert!(
        matches!(zero_threshold, Err(crate::Error::Config(ref m)) if m.contains("circuit_failure_threshold"))
    );
    let after = db.guardrail_limits();
    assert_eq!(
        after.timeout_ms, before.timeout_ms,
        "a refused edit changes nothing"
    );
    assert_eq!(after.rate_limit_qps, before.rate_limit_qps);
    let coll = db.get_vector_collection("docs").unwrap();
    assert_eq!(coll.guard_rails().limits().timeout_ms, before.timeout_ms);
}
