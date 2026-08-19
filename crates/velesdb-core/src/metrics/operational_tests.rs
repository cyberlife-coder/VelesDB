use super::*;

#[test]
fn test_operational_metrics_counters() {
    let metrics = OperationalMetrics::new();

    metrics.record_vector_query();
    metrics.record_vector_query();
    metrics.record_graph_query();
    metrics.record_hybrid_query();
    metrics.inc_errors();

    assert_eq!(metrics.queries_total.load(Ordering::Relaxed), 4);
    assert_eq!(metrics.vector_queries.load(Ordering::Relaxed), 2);
    assert_eq!(metrics.graph_queries.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.hybrid_queries.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.query_errors.load(Ordering::Relaxed), 1);
}

#[test]
fn test_operational_metrics_gauges() {
    let metrics = OperationalMetrics::new();

    metrics.set_documents(1000);
    metrics.set_index_size(1024 * 1024);
    metrics.inc_connections();
    metrics.inc_connections();
    metrics.dec_connections();

    assert_eq!(metrics.documents_total.load(Ordering::Relaxed), 1000);
    assert_eq!(
        metrics.index_size_bytes.load(Ordering::Relaxed),
        1024 * 1024
    );
    assert_eq!(metrics.active_connections.load(Ordering::Relaxed), 1);
}

#[test]
fn test_operational_metrics_prometheus_export() {
    let metrics = OperationalMetrics::new();
    metrics.record_vector_query();
    metrics.set_documents(100);

    let output = metrics.export_prometheus();

    assert!(output.contains("velesdb_queries_total"));
    assert!(output.contains("velesdb_documents_total 100"));
    assert!(output.contains("# TYPE"));
    assert!(output.contains("# HELP"));
}

#[test]
fn test_operational_metrics_new_arc() {
    let metrics = OperationalMetrics::new_arc();
    metrics.record_vector_query();

    // Clone Arc and verify shared state
    let metrics2 = Arc::clone(&metrics);
    metrics2.record_vector_query();

    assert_eq!(metrics.queries_total.load(Ordering::Relaxed), 2);
}

#[test]
fn test_rate_limited_request_increments_status_rate_limited() {
    let metrics = OperationalMetrics::new();

    // Simulate 3 queries: 1 succeeds, 1 errors, 1 rate-limited
    metrics.record_vector_query(); // queries_total = 1
    metrics.record_vector_query(); // queries_total = 2
    metrics.record_vector_query(); // queries_total = 3
    metrics.inc_errors(); // 1 error
    metrics.inc_rate_limited(); // 1 rate-limited

    assert_eq!(metrics.queries_total.load(Ordering::Relaxed), 3);
    assert_eq!(metrics.query_errors.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.query_rate_limited.load(Ordering::Relaxed), 1);

    let output = metrics.export_prometheus();

    // success = total - errors - rate_limited = 3 - 1 - 1 = 1
    assert!(
        output.contains("velesdb_queries_total{status=\"success\"} 1"),
        "expected success=1 in:\n{output}"
    );
    assert!(
        output.contains("velesdb_queries_total{status=\"error\"} 1"),
        "expected error=1 in:\n{output}"
    );
    assert!(
        output.contains("velesdb_queries_total{status=\"rate_limited\"} 1"),
        "expected rate_limited=1 in:\n{output}"
    );
}
