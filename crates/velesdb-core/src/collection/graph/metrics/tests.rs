//! Tests for graph performance metrics.

use super::*;
use std::time::Duration;

#[test]
fn test_latency_histogram_observe() {
    let hist = LatencyHistogram::new();

    hist.observe(Duration::from_micros(500)); // <1ms bucket
    hist.observe(Duration::from_millis(3)); // <5ms bucket
    hist.observe(Duration::from_millis(75)); // <100ms bucket

    assert_eq!(hist.count(), 3);
    let buckets = hist.bucket_counts();
    assert_eq!(buckets[0], 1); // <1ms
    assert_eq!(buckets[1], 1); // <5ms
    assert_eq!(buckets[4], 1); // <100ms
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

    // Test >10s bucket
    hist.observe(Duration::from_secs(15));

    let buckets = hist.bucket_counts();
    assert_eq!(buckets[9], 1); // ≥10s bucket
}
