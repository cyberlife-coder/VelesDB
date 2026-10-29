//! The `/metrics` counters core owns are fed by the paths they describe.
//!
//! `velesdb_limits_exceeded_total`, `velesdb_rate_limit_requests_total`,
//! `velesdb_match_guardrail_hits_total` and
//! `velesdb_match_similarity_queries_total` were declared and exported with
//! no caller, so they read 0 forever. Every collector here is process-wide:
//! the tests read deltas, which the single-threaded test run keeps exact.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tempfile::TempDir;

use crate::collection::graph::GraphEdge;
use crate::collection::search::query::match_metrics::global_match_metrics;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::guardrails::{GuardRails, QueryContext, QueryLimits, RateLimiter};
use crate::metrics::global_guardrails_metrics;
use crate::point::Point;
use crate::velesql::Parser;

fn delta(counter: &AtomicU64, run: impl FnOnce()) -> u64 {
    let before = counter.load(Ordering::Relaxed);
    run();
    counter.load(Ordering::Relaxed) - before
}

#[test]
fn every_query_context_violation_feeds_limits_exceeded() {
    let metrics = global_guardrails_metrics();
    let ctx = QueryContext::new(QueryLimits {
        max_depth: 1,
        max_cardinality: 1,
        memory_limit_bytes: 1,
        ..QueryLimits::default()
    });

    assert_eq!(
        delta(&metrics.depth_exceeded, || drop(ctx.check_depth(2))),
        1
    );
    assert_eq!(
        delta(&metrics.cardinality_exceeded, || drop(
            ctx.check_cardinality(2)
        )),
        1
    );
    assert_eq!(
        delta(&metrics.memory_exceeded, || drop(ctx.check_memory(2))),
        1
    );

    let within = QueryContext::new(QueryLimits::default());
    assert_eq!(
        delta(&metrics.depth_exceeded, || drop(within.check_depth(1))),
        0,
        "a check within its limit records nothing"
    );
}

#[test]
fn a_timed_out_query_context_feeds_limits_exceeded() {
    let ctx = QueryContext::new(QueryLimits::default().with_timeout_ms(1));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let metrics = global_guardrails_metrics();
    assert_eq!(
        delta(&metrics.timeout_exceeded, || drop(ctx.check_timeout())),
        1
    );
}

#[test]
fn rate_limiter_decisions_feed_rate_limit_requests() {
    let metrics = global_guardrails_metrics();
    let limiter = RateLimiter::new(1);

    assert_eq!(
        delta(&metrics.rate_limit_allowed, || drop(limiter.check("c"))),
        1
    );
    assert_eq!(
        delta(&metrics.rate_limit_rejected, || drop(limiter.check("c"))),
        1
    );
}

/// Three `:Doc` nodes, 1 -> 2 -> 3, each with a 2-d vector.
fn doc_chain(dir: &TempDir) -> Collection {
    let col = Collection::create(dir.path().join("docs"), 2, DistanceMetric::Cosine)
        .expect("test: create");
    let doc = || Some(serde_json::json!({"_labels": ["Doc"]}));
    col.upsert(vec![
        Point::new(1, vec![1.0, 0.0], doc()),
        Point::new(2, vec![0.7, 0.7], doc()),
        Point::new(3, vec![0.0, 1.0], doc()),
    ])
    .expect("test: upsert");
    col.add_edge(GraphEdge::new(10, 1, 2, "LINK").expect("edge"))
        .expect("test: edge");
    col.add_edge(GraphEdge::new(11, 2, 3, "LINK").expect("edge"))
        .expect("test: edge");
    col
}

#[test]
fn a_match_with_similarity_feeds_similarity_queries() {
    let dir = TempDir::new().expect("test: temp dir");
    let col = doc_chain(&dir);
    let query = Parser::parse(
        "MATCH (a:Doc)-[:LINK]->(b:Doc) WHERE similarity(a, $v) > 0.0 RETURN a, b LIMIT 10",
    )
    .expect("test: parse");
    let params = HashMap::from([("v".to_string(), serde_json::json!([1.0, 0.0]))]);
    let plain =
        Parser::parse("MATCH (a:Doc)-[:LINK]->(b:Doc) RETURN b LIMIT 10").expect("test: parse");

    let counter = &global_match_metrics().similarity_queries;
    assert_eq!(
        delta(counter, || {
            col.execute_query(&query, &params)
                .expect("test: similarity MATCH");
        }),
        1
    );
    assert_eq!(
        delta(counter, || {
            col.execute_query(&plain, &HashMap::new())
                .expect("test: plain MATCH");
        }),
        0
    );
}

#[test]
fn a_match_stopped_by_a_guard_rail_feeds_guardrail_hits() {
    let dir = TempDir::new().expect("test: temp dir");
    let mut col = doc_chain(&dir);
    col.runtime.guard_rails = Arc::new(GuardRails::with_limits(QueryLimits {
        max_cardinality: 1,
        ..QueryLimits::default()
    }));
    let query =
        Parser::parse("MATCH (a:Doc)-[:LINK]->(b:Doc) RETURN b LIMIT 10").expect("test: parse");

    let hits = delta(&global_match_metrics().guard_rail_hits, || {
        assert!(col.execute_query(&query, &HashMap::new()).is_err());
    });
    assert_eq!(hits, 1, "two rows over a cardinality cap of one");

    let ordered = query.match_clause.as_ref().expect("test: MATCH clause");
    let hits = delta(&global_match_metrics().guard_rail_hits, || {
        assert!(col.match_query_ordered(ordered, &HashMap::new()).is_err());
    });
    assert_eq!(hits, 1, "the ordered surface counts the same stop");
}
