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

    assert!(metrics.latency_buckets[0].load(Ordering::Relaxed) >= 1);
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
