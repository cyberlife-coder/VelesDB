use super::*;

#[test]
fn test_slow_query_is_slow() {
    let logger = SlowQueryLogger::new(Duration::from_millis(100));

    assert!(!logger.is_slow(Duration::from_millis(50)));
    assert!(logger.is_slow(Duration::from_millis(100)));
    assert!(logger.is_slow(Duration::from_millis(150)));
}

#[test]
fn test_slow_query_disabled() {
    let logger = SlowQueryLogger::disabled();

    assert!(!logger.is_slow(Duration::from_secs(1000)));
}

#[test]
fn test_slow_query_sanitize() {
    let query = r#"SELECT * FROM users WHERE name = "John Doe" AND age > 30"#;
    let sanitized = SlowQueryLogger::sanitize_query(query);

    assert!(!sanitized.contains("John Doe"));
    assert!(sanitized.contains('?'));
    assert!(sanitized.contains("SELECT"));
    assert!(sanitized.contains("age > 30"));
}

#[test]
fn test_slow_query_sanitize_single_quotes() {
    let query = "SELECT * FROM docs WHERE title = 'Secret Document'";
    let sanitized = SlowQueryLogger::sanitize_query(query);

    assert!(!sanitized.contains("Secret Document"));
    assert!(sanitized.contains('?'));
}

#[test]
fn test_query_stats_default() {
    let stats = QueryStats::default();

    assert_eq!(stats.rows_scanned, 0);
    assert_eq!(stats.nodes_visited, 0);
    assert_eq!(stats.vectors_compared, 0);
    assert!(stats.collection.is_empty());
}

#[test]
fn test_query_phase_span_names() {
    assert_eq!(QueryPhase::Parse.span_name(), "parse");
    assert_eq!(QueryPhase::Plan.span_name(), "plan");
    assert_eq!(QueryPhase::VectorSearch.span_name(), "vector_search");
    assert_eq!(QueryPhase::GraphTraversal.span_name(), "graph_traversal");
    assert_eq!(QueryPhase::ScoreFusion.span_name(), "score_fusion");
    assert_eq!(QueryPhase::Filter.span_name(), "filter");
    assert_eq!(QueryPhase::Sort.span_name(), "sort");
}

#[test]
fn test_span_builder() {
    let builder = SpanBuilder::new("test_collection")
        .with_rows(100)
        .with_context("test context");

    assert_eq!(builder.collection, "test_collection");
    assert_eq!(builder.rows_processed, 100);
    assert_eq!(builder.context, "test context");
}

#[test]
fn test_span_builder_creates_span() {
    let builder = SpanBuilder::new("my_collection").with_rows(50);
    // Span creation should not panic (span may be disabled without subscriber)
    let _span = builder.span(QueryPhase::VectorSearch);
}

#[test]
fn test_duration_histogram_observe() {
    let histogram = DurationHistogram::new();

    histogram.observe(0.002); // 2ms -> bucket 0.005
    histogram.observe(0.02); // 20ms -> bucket 0.05
    histogram.observe(0.5); // 500ms -> bucket 0.5

    assert_eq!(histogram.count.load(Ordering::Relaxed), 3);
    assert!(histogram.sum.load(Ordering::Relaxed) > 0);
}

#[test]
fn test_duration_histogram_prometheus_export() {
    let histogram = DurationHistogram::new();
    histogram.observe(0.01);
    histogram.observe(0.1);

    let output = histogram.export_prometheus(
        "velesdb_query_duration_seconds",
        "Query duration in seconds",
    );

    assert!(output.contains("velesdb_query_duration_seconds_bucket"));
    assert!(output.contains("velesdb_query_duration_seconds_sum"));
    assert!(output.contains("velesdb_query_duration_seconds_count 2"));
    assert!(output.contains("le=\"+Inf\""));
}
