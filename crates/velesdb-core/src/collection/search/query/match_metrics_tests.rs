//! Tests for `match_metrics` module - MATCH query metrics collection.

use super::match_metrics::*;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[test]
fn test_metrics_record_success() {
    let metrics = MatchMetrics::new();
    metrics.record_success(Duration::from_millis(10), 5, 3);

    assert_eq!(metrics.total_queries.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.successful_queries.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.total_results.load(Ordering::Relaxed), 5);
}

#[test]
fn test_metrics_record_failure() {
    let metrics = MatchMetrics::new();
    metrics.record_failure(Duration::from_millis(100));

    assert_eq!(metrics.total_queries.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.failed_queries.load(Ordering::Relaxed), 1);
}

#[test]
fn test_metrics_success_rate() {
    let metrics = MatchMetrics::new();
    metrics.record_success(Duration::from_millis(10), 5, 3);
    metrics.record_success(Duration::from_millis(10), 5, 3);
    metrics.record_failure(Duration::from_millis(10));

    let rate = metrics.success_rate();
    assert!((rate - 0.6666).abs() < 0.01);
}

#[test]
fn test_metrics_latency_buckets() {
    let metrics = MatchMetrics::new();
    metrics.record_success(Duration::from_micros(500), 1, 1);
    metrics.record_success(Duration::from_millis(3), 1, 1);
    metrics.record_success(Duration::from_millis(50), 1, 1);

    assert_eq!(
        metrics.latency_buckets[0].load(Ordering::Relaxed),
        1,
        "500us -> bucket 0 (le=1ms)"
    );
    assert_eq!(
        metrics.latency_buckets[1].load(Ordering::Relaxed),
        1,
        "3ms -> bucket 1 (le=5ms)"
    );
    assert_eq!(
        metrics.latency_buckets[4].load(Ordering::Relaxed),
        1,
        "50ms -> bucket 4 (le=50ms is inclusive)"
    );
    for (i, b) in metrics.latency_buckets.iter().enumerate() {
        if i != 0 && i != 1 && i != 4 {
            assert_eq!(b.load(Ordering::Relaxed), 0, "bucket {i} should be empty");
        }
    }
}

#[test]
fn test_prometheus_output() {
    let metrics = MatchMetrics::new();
    metrics.record_success(Duration::from_millis(10), 5, 3);

    let output = metrics.to_prometheus();
    assert!(output.contains("velesdb_match_queries_total 1"));
    assert!(output.contains("velesdb_match_queries_success_total 1"));
    assert!(output.contains("velesdb_match_results_total 5"));
}

#[test]
fn test_query_timer_success() {
    let metrics = MatchMetrics::new();
    {
        let timer = QueryTimer::new(&metrics);
        std::thread::sleep(Duration::from_millis(1));
        timer.success(10, 2);
    }
    assert_eq!(metrics.successful_queries.load(Ordering::Relaxed), 1);
}

#[test]
fn test_query_timer_drop_counts_as_failure() {
    let metrics = MatchMetrics::new();
    {
        let _timer = QueryTimer::new(&metrics);
    }
    assert_eq!(metrics.failed_queries.load(Ordering::Relaxed), 1);
}

/// A MATCH latency landing exactly on a bucket bound belongs to that bucket.
///
/// `to_prometheus` exports these counts as `le="{bound}"`, and Prometheus
/// defines `le` as **less than or equal**. Bucketing with `ms < bound` moved an
/// exact-bound observation one bucket up, so a query that took exactly 25 ms
/// was reported as *not* being ≤ 25 ms.
#[test]
fn a_match_latency_on_a_bucket_bound_lands_in_that_bucket() {
    let metrics = MatchMetrics::new();
    metrics.record_success(Duration::from_millis(25), 1, 1);

    let counts: Vec<u64> = metrics
        .latency_buckets
        .iter()
        .map(|b| b.load(Ordering::Relaxed))
        .collect();
    let index = LATENCY_BUCKETS_MS
        .iter()
        .position(|&b| b == 25)
        .expect("25 is a declared bound");
    assert_eq!(
        counts[index], 1,
        "25 ms must fall in le=\"25\" (index {index}), got {counts:?}"
    );
}

/// Every declared bound is inclusive, and the overflow bucket keeps its role.
#[test]
fn every_match_latency_bound_is_inclusive() {
    for (index, bound) in LATENCY_BUCKETS_MS.into_iter().enumerate() {
        let metrics = MatchMetrics::new();
        metrics.record_success(Duration::from_millis(bound), 1, 1);
        let count = metrics.latency_buckets[index].load(Ordering::Relaxed);
        assert_eq!(
            count, 1,
            "{bound} ms must land in le=\"{bound}\" (index {index})"
        );
    }

    let metrics = MatchMetrics::new();
    metrics.record_success(Duration::from_millis(5_001), 1, 1);
    assert_eq!(
        metrics.latency_buckets[LATENCY_BUCKETS_MS.len()].load(Ordering::Relaxed),
        1,
        "a latency above the last bound still overflows"
    );
}
