use super::*;

#[test]
fn test_traversal_metrics_record() {
    let metrics = TraversalMetrics::new();

    metrics.record_traversal(100, 3, 250);
    metrics.record_traversal(50, 2, 100);

    assert_eq!(metrics.traversal_count.load(Ordering::Relaxed), 2);
    assert_eq!(metrics.nodes_visited_total.load(Ordering::Relaxed), 150);
    assert_eq!(metrics.edges_scanned_total.load(Ordering::Relaxed), 350);
    assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 3);
}

#[test]
fn test_traversal_metrics_max_depth_updates() {
    let metrics = TraversalMetrics::new();

    metrics.record_traversal(10, 2, 20);
    assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 2);

    metrics.record_traversal(10, 5, 20);
    assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 5);

    // Smaller depth doesn't decrease max
    metrics.record_traversal(10, 3, 20);
    assert_eq!(metrics.max_depth_reached.load(Ordering::Relaxed), 5);
}

#[test]
fn test_traversal_metrics_prometheus_export() {
    let metrics = TraversalMetrics::new();
    metrics.record_traversal(100, 5, 200);

    let output = metrics.export_prometheus();

    assert!(output.contains("velesdb_traversal_nodes_visited_total 100"));
    assert!(output.contains("velesdb_traversal_max_depth 5"));
    assert!(output.contains("velesdb_traversal_edges_scanned_total 200"));
}

#[test]
fn test_guardrails_record_limits() {
    let metrics = GuardRailsMetrics::new();

    metrics.record_limit_exceeded(LimitType::Timeout);
    metrics.record_limit_exceeded(LimitType::Timeout);
    metrics.record_limit_exceeded(LimitType::Depth);
    metrics.record_limit_exceeded(LimitType::Memory);

    assert_eq!(metrics.timeout_exceeded.load(Ordering::Relaxed), 2);
    assert_eq!(metrics.depth_exceeded.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.memory_exceeded.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.cardinality_exceeded.load(Ordering::Relaxed), 0);
}

#[test]
fn test_guardrails_rate_limit() {
    let metrics = GuardRailsMetrics::new();

    metrics.record_rate_limit(true);
    metrics.record_rate_limit(true);
    metrics.record_rate_limit(false);

    assert_eq!(metrics.rate_limit_allowed.load(Ordering::Relaxed), 2);
    assert_eq!(metrics.rate_limit_rejected.load(Ordering::Relaxed), 1);
}

#[test]
fn test_guardrails_prometheus_export() {
    let metrics = GuardRailsMetrics::new();
    metrics.record_limit_exceeded(LimitType::Timeout);
    metrics.record_rate_limit(false);
    metrics.record_like_guardrail_rejected();
    metrics.record_parser_depth_limit_rejected();
    metrics.record_invalid_offset_read_error();
    metrics.record_cache_collision_fallback();

    let output = metrics.export_prometheus();

    assert!(output.contains("velesdb_limits_exceeded_total"));
    assert!(output.contains("limit_type=\"timeout\""));
    assert!(output.contains("velesdb_rate_limit_requests_total"));
    assert!(output.contains("velesdb_like_guardrail_rejected_total 1"));
    assert!(output.contains("velesdb_parser_depth_limit_rejected_total 1"));
    assert!(output.contains("velesdb_invalid_offset_read_errors_total 1"));
    assert!(output.contains("velesdb_cache_collision_fallback_total 1"));
}

#[test]
fn test_limit_type_as_str() {
    assert_eq!(LimitType::Timeout.as_str(), "timeout");
    assert_eq!(LimitType::Depth.as_str(), "depth");
    assert_eq!(LimitType::Cardinality.as_str(), "cardinality");
    assert_eq!(LimitType::Memory.as_str(), "memory");
}
