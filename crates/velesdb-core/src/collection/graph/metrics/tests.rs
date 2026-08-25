//! Tests for graph performance metrics.

use super::*;
use std::time::Duration;

#[test]
fn test_latency_histogram_observe() {
    let hist = LatencyHistogram::new();

    hist.observe(Duration::from_micros(500)); // le=1ms bucket
    hist.observe(Duration::from_millis(3)); // le=5ms bucket
    hist.observe(Duration::from_millis(75)); // le=100ms bucket

    assert_eq!(hist.count(), 3);
    let buckets = hist.bucket_counts();
    assert_eq!(buckets[0], 1); // le=1ms
    assert_eq!(buckets[1], 1); // le=5ms
    assert_eq!(buckets[4], 1); // le=100ms
}

#[test]
fn test_latency_histogram_avg() {
    let hist = LatencyHistogram::new();

    hist.observe(Duration::from_millis(10));
    hist.observe(Duration::from_millis(20));
    hist.observe(Duration::from_millis(30));

    // Average should be 20ms = 20_000_000 ns
    let avg = hist.avg_ns();
    assert!((avg - 20_000_000.0).abs() < 1000.0);
}

#[test]
fn test_latency_histogram_reset() {
    let hist = LatencyHistogram::new();

    hist.observe(Duration::from_millis(10));
    assert_eq!(hist.count(), 1);

    hist.reset();
    assert_eq!(hist.count(), 0);
    assert_eq!(hist.sum_ns(), 0);
}

#[test]
fn test_graph_metrics_edge_insert() {
    let metrics = GraphMetrics::new();

    metrics.record_edge_insert(Duration::from_millis(5));
    metrics.record_edge_insert(Duration::from_millis(10));

    assert_eq!(metrics.edges_total(), 2);
    assert_eq!(metrics.edge_inserts_total(), 2);
    assert_eq!(metrics.edge_insert_latency.count(), 2);
}

#[test]
fn test_graph_metrics_node_operations() {
    let metrics = GraphMetrics::new();

    metrics.record_node_insert();
    metrics.record_node_insert();
    metrics.record_node_delete();

    assert_eq!(metrics.nodes_total(), 1);
    assert_eq!(metrics.node_inserts_total(), 2);
}

#[test]
fn test_graph_metrics_traversal() {
    let metrics = GraphMetrics::new();

    metrics.record_traversal(Duration::from_millis(50), 1000);
    metrics.record_traversal(Duration::from_millis(100), 2000);

    assert_eq!(metrics.traversals_total(), 2);
    assert_eq!(metrics.traversal_nodes_visited(), 3000);
    assert_eq!(metrics.traversal_latency.count(), 2);
}

#[test]
fn test_graph_metrics_prometheus_format() {
    let metrics = GraphMetrics::new();

    metrics.record_edge_insert(Duration::from_millis(5));
    metrics.record_node_insert();
    metrics.record_traversal(Duration::from_millis(10), 100);

    let output = metrics.to_prometheus();

    // Verify Prometheus format
    assert!(output.contains("# HELP velesdb_graph_nodes_total"));
    assert!(output.contains("# TYPE velesdb_graph_nodes_total gauge"));
    assert!(output.contains("velesdb_graph_nodes_total 1"));
    assert!(output.contains("velesdb_graph_edges_total 1"));
    assert!(output.contains("velesdb_graph_edge_insert_duration_seconds_bucket"));
}

#[test]
fn test_graph_metrics_reset() {
    let metrics = GraphMetrics::new();

    metrics.record_edge_insert(Duration::from_millis(5));
    metrics.record_node_insert();

    metrics.reset();

    assert_eq!(metrics.edges_total(), 0);
    assert_eq!(metrics.nodes_total(), 0);
    assert_eq!(metrics.edge_insert_latency.count(), 0);
}

#[test]
fn test_latency_histogram_empty_avg() {
    let hist = LatencyHistogram::new();
    assert!(hist.avg_ns().abs() < f64::EPSILON);
}

#[test]
fn test_latency_histogram_large_duration() {
    let hist = LatencyHistogram::new();

    // Test the >10s overflow bucket
    hist.observe(Duration::from_secs(15));

    let buckets = hist.bucket_counts();
    assert_eq!(buckets[9], 1); // >10s bucket
}

/// An observation landing exactly on a bucket bound belongs to that bucket.
///
/// The counts are exported with `le="{bound}"`, and Prometheus defines `le` as
/// **less than or equal**. Bucketing with `ms < bound` pushed an exact-bound
/// observation into the next bucket, so `le="5"` under-reported every request
/// that took exactly 5 ms — the scrape said "no request was ≤ 5 ms" about a
/// request that was.
#[test]
fn an_observation_on_a_bucket_bound_lands_in_that_bucket() {
    let hist = LatencyHistogram::new();
    hist.observe(Duration::from_millis(5));

    let counts = hist.bucket_counts();
    assert_eq!(
        counts[1], 1,
        "5 ms must fall in le=\"5\" (index 1), got {counts:?}"
    );
    assert_eq!(counts[2], 0, "it must not spill into le=\"10\"");
}

/// Every bound is inclusive, not just the one probed above.
#[test]
fn every_bucket_bound_is_inclusive() {
    for (index, bound) in BUCKET_BOUNDS_MS.into_iter().enumerate() {
        let hist = LatencyHistogram::new();
        hist.observe(Duration::from_millis(bound));
        let counts = hist.bucket_counts();
        assert_eq!(
            counts[index], 1,
            "{bound} ms must land in le=\"{bound}\" (index {index}), got {counts:?}"
        );
    }
}

/// A value above the last bound still lands in the overflow bucket.
#[test]
fn an_observation_above_the_last_bound_overflows() {
    let hist = LatencyHistogram::new();
    hist.observe(Duration::from_millis(10_001));
    assert_eq!(hist.bucket_counts()[9], 1, "overflow bucket keeps its role");
}

/// The exported `le` labels are the recorded bounds in seconds, and the
/// cumulative counts are consistent with the bucket an observation landed in.
///
/// The label list used to be a hand-written array of strings sitting next to
/// `BUCKET_BOUNDS_MS`. Two sources of truth for one fact drift silently: a
/// bound added or moved on one side would have relabelled every sample on the
/// other. Deriving the labels removes the possibility; this locks the result.
#[test]
fn exported_bucket_labels_are_the_recorded_bounds_in_seconds() {
    let metrics = GraphMetrics::new();
    metrics.record_edge_insert(Duration::from_millis(5));

    let output = metrics.to_prometheus();

    for (index, bound_ms) in BUCKET_BOUNDS_MS.into_iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let seconds = bound_ms as f64 / 1000.0;
        // 5 ms sits on the second bound, so every bucket from there on is
        // cumulative 1 and every earlier one is 0.
        let expected = u64::from(index >= 1);
        let line = format!(
            "velesdb_graph_edge_insert_duration_seconds_bucket{{le=\"{seconds}\"}} {expected}"
        );
        assert!(
            output.contains(&line),
            "missing or miscounted bucket line: {line}\n--- output ---\n{output}"
        );
    }

    assert!(
        output.contains("velesdb_graph_edge_insert_duration_seconds_bucket{le=\"+Inf\"} 1"),
        "the overflow bucket must close the cumulative series\n--- output ---\n{output}"
    );
}
