//! Tests for collection statistics (EPIC-046 US-001).

use super::*;

#[test]
fn test_collection_stats_new() {
    let stats = CollectionStats::new();
    assert_eq!(stats.row_count, 0);
    assert_eq!(stats.deleted_count, 0);
    assert!(stats.column_stats.is_empty());
}

#[test]
fn test_collection_stats_with_counts() {
    let stats = CollectionStats::with_counts(10_000, 500);
    assert_eq!(stats.row_count, 10_000);
    assert_eq!(stats.deleted_count, 500);
    assert_eq!(stats.live_row_count(), 9_500);
}

#[test]
fn test_deletion_ratio() {
    let stats = CollectionStats::with_counts(1000, 100);
    assert!((stats.deletion_ratio() - 0.1).abs() < 0.001);

    let empty = CollectionStats::new();
    assert!((empty.deletion_ratio() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_estimate_selectivity_with_column() {
    let mut stats = CollectionStats::with_counts(10_000, 0);
    stats.column_stats.insert(
        "category".to_string(),
        ColumnStats::new("category").with_distinct_count(50),
    );

    let selectivity = stats.estimate_selectivity("category");
    assert!((selectivity - 0.02).abs() < 0.001); // 1/50 = 0.02
}

#[test]
fn test_estimate_selectivity_unknown_column() {
    let stats = CollectionStats::with_counts(10_000, 0);
    let selectivity = stats.estimate_selectivity("unknown");
    assert!((selectivity - 0.1).abs() < 0.001); // Default 10%
}

#[test]
fn test_column_stats_builder() {
    let col = ColumnStats::new("age")
        .with_distinct_count(100)
        .with_null_count(5);

    assert_eq!(col.name, "age");
    assert_eq!(col.distinct_count, 100);
    assert_eq!(col.null_count, 5);
}

#[test]
fn test_index_stats_builder() {
    let idx = IndexStats::new("hnsw_embedding", "HNSW")
        .with_entry_count(10_000)
        .with_depth(4);

    assert_eq!(idx.name, "hnsw_embedding");
    assert_eq!(idx.index_type, "HNSW");
    assert_eq!(idx.entry_count, 10_000);
    assert_eq!(idx.depth, 4);
}

#[test]
fn test_stats_collector_basic() {
    let mut collector = StatsCollector::new();
    collector.set_row_count(10_000);
    collector.set_deleted_count(100);
    collector.set_total_size(2_560_000); // 256 bytes avg

    let stats = collector.build();

    assert_eq!(stats.row_count, 10_000);
    assert_eq!(stats.deleted_count, 100);
    assert_eq!(stats.avg_row_size_bytes, 256);
    assert!(stats.last_analyzed_epoch_ms.is_some());
}

#[test]
fn test_stats_collector_with_columns_and_indexes() {
    let mut collector = StatsCollector::new();
    collector.set_row_count(5_000);

    collector.add_column_stats(ColumnStats::new("category").with_distinct_count(20));
    collector.add_column_stats(ColumnStats::new("status").with_distinct_count(5));

    collector
        .add_index_stats(IndexStats::new("idx_category", "PropertyIndex").with_entry_count(5_000));

    let stats = collector.build();

    assert_eq!(stats.column_stats.len(), 2);
    assert_eq!(stats.index_stats.len(), 1);
    assert_eq!(
        stats.column_stats.get("category").unwrap().distinct_count,
        20
    );
}

#[test]
fn test_stats_serialization() {
    let mut stats = CollectionStats::with_counts(1000, 50);
    stats.column_stats.insert(
        "name".to_string(),
        ColumnStats::new("name").with_distinct_count(800),
    );
    stats.mark_analyzed();

    // Test JSON serialization
    let json = serde_json::to_string(&stats).expect("serialize");
    let deserialized: CollectionStats = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(deserialized.row_count, 1000);
    assert_eq!(deserialized.deleted_count, 50);
    assert_eq!(
        deserialized
            .column_stats
            .get("name")
            .unwrap()
            .distinct_count,
        800
    );
}
