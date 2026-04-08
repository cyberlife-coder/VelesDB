//! Tests for collection statistics (EPIC-046 US-001).

use super::*;
use histogram::{Histogram, HistogramBucket};

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

// --- Histogram core method tests ---

/// Helper: builds a 3-bucket histogram for testing.
fn make_test_histogram() -> Histogram {
    Histogram {
        buckets: vec![
            HistogramBucket {
                lower_bound: 0.0,
                upper_bound: 10.0,
                count: 100,
                distinct_count: 10,
            },
            HistogramBucket {
                lower_bound: 10.0,
                upper_bound: 20.0,
                count: 200,
                distinct_count: 20,
            },
            HistogramBucket {
                lower_bound: 20.0,
                upper_bound: 30.0,
                count: 300,
                distinct_count: 15,
            },
        ],
        total_count: 600,
        incremental_updates: 0,
        stale: false,
    }
}

#[test]
fn test_find_bucket_in_range() {
    let h = make_test_histogram();
    assert_eq!(h.find_bucket(5.0), Some(0));
    assert_eq!(h.find_bucket(0.0), Some(0)); // lower_bound inclusive
    assert_eq!(h.find_bucket(10.0), Some(1)); // next bucket starts at 10.0
    assert_eq!(h.find_bucket(25.0), Some(2));
}

#[test]
fn test_find_bucket_outside_range() {
    let h = make_test_histogram();
    assert_eq!(h.find_bucket(-1.0), None);
    assert_eq!(h.find_bucket(30.0), None); // upper_bound exclusive
    assert_eq!(h.find_bucket(100.0), None);
}

#[test]
fn test_find_bucket_empty_histogram() {
    let h = Histogram::default();
    assert_eq!(h.find_bucket(5.0), None);
}

#[test]
fn test_eq_selectivity_in_bucket() {
    let h = make_test_histogram();
    // Bucket 0: count=100, distinct=10, total=600
    // Expected: 100 / (10 * 600) = 100/6000 ≈ 0.01667
    let sel = h.estimate_eq_selectivity(5.0);
    assert!((sel - 100.0 / 6000.0).abs() < 1e-10);
}

#[test]
fn test_eq_selectivity_outside_range() {
    let h = make_test_histogram();
    // Outside: 1/600 ≈ 0.001667
    let sel = h.estimate_eq_selectivity(50.0);
    assert!((sel - 1.0 / 600.0).abs() < 1e-10);
}

#[test]
fn test_eq_selectivity_zero_total() {
    let h = Histogram {
        total_count: 0,
        ..Default::default()
    };
    assert!((h.estimate_eq_selectivity(5.0) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_eq_selectivity_zero_distinct() {
    let h = Histogram {
        buckets: vec![HistogramBucket {
            lower_bound: 0.0,
            upper_bound: 10.0,
            count: 50,
            distinct_count: 0,
        }],
        total_count: 50,
        ..Default::default()
    };
    let sel = h.estimate_eq_selectivity(5.0);
    assert!((sel - 1.0 / 50.0).abs() < 1e-10);
}

#[test]
fn test_lt_selectivity_below_first() {
    let h = make_test_histogram();
    assert!((h.estimate_lt_selectivity(-1.0) - 0.0).abs() < f64::EPSILON);
    assert!((h.estimate_lt_selectivity(0.0) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_lt_selectivity_above_last() {
    let h = make_test_histogram();
    assert!((h.estimate_lt_selectivity(30.0) - 1.0).abs() < f64::EPSILON);
    assert!((h.estimate_lt_selectivity(100.0) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_lt_selectivity_midpoint() {
    let h = make_test_histogram();
    // value=15.0 → bucket 0 fully below (100), bucket 1 partial: (15-10)/(20-10)=0.5 → 200*0.5=100
    // total below = 200, selectivity = 200/600 ≈ 0.3333
    let sel = h.estimate_lt_selectivity(15.0);
    assert!((sel - 200.0 / 600.0).abs() < 1e-10);
}

#[test]
fn test_lt_selectivity_empty() {
    let h = Histogram::default();
    assert!((h.estimate_lt_selectivity(5.0) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_range_selectivity_full_range() {
    let h = make_test_histogram();
    // Range encompasses entire histogram
    assert!((h.estimate_range_selectivity(-10.0, 50.0) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_range_selectivity_outside() {
    let h = make_test_histogram();
    assert!((h.estimate_range_selectivity(50.0, 60.0) - 0.0).abs() < f64::EPSILON);
    assert!((h.estimate_range_selectivity(-20.0, -10.0) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_range_selectivity_inverted() {
    let h = make_test_histogram();
    assert!((h.estimate_range_selectivity(20.0, 10.0) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_range_selectivity_partial() {
    let h = make_test_histogram();
    // [10.0, 20.0] = exactly bucket 1 → 200/600 ≈ 0.3333
    let sel = h.estimate_range_selectivity(10.0, 20.0);
    assert!((sel - 200.0 / 600.0).abs() < 1e-10);
}

#[test]
fn test_increment_bucket_updates_count() {
    let mut h = make_test_histogram();
    h.increment_bucket(5.0);
    assert_eq!(h.buckets[0].count, 101);
    assert_eq!(h.incremental_updates, 1);
    assert!(!h.stale);
}

#[test]
fn test_increment_bucket_outside_noop() {
    let mut h = make_test_histogram();
    h.increment_bucket(50.0);
    assert_eq!(h.buckets[0].count, 100);
    assert_eq!(h.incremental_updates, 0);
}

#[test]
fn test_decrement_bucket_updates_count() {
    let mut h = make_test_histogram();
    h.decrement_bucket(5.0);
    assert_eq!(h.buckets[0].count, 99);
    assert_eq!(h.incremental_updates, 1);
}

#[test]
fn test_decrement_bucket_floors_at_zero() {
    let mut h = Histogram {
        buckets: vec![HistogramBucket {
            lower_bound: 0.0,
            upper_bound: 10.0,
            count: 0,
            distinct_count: 1,
        }],
        total_count: 100,
        ..Default::default()
    };
    h.decrement_bucket(5.0);
    assert_eq!(h.buckets[0].count, 0);
    assert_eq!(h.incremental_updates, 1);
}

#[test]
fn test_staleness_threshold() {
    let mut h = Histogram {
        buckets: vec![HistogramBucket {
            lower_bound: 0.0,
            upper_bound: 100.0,
            count: 100,
            distinct_count: 50,
        }],
        total_count: 100,
        incremental_updates: 0,
        stale: false,
    };
    // 20% of 100 = 20. Need > 20 updates to trigger staleness.
    for _ in 0..20 {
        h.increment_bucket(50.0);
    }
    assert!(!h.stale);
    h.increment_bucket(50.0); // 21st update → > 100/5 = 20
    assert!(h.stale);
}

// --- HistogramBuilder tests ---

use histogram::HistogramBuilder;

#[test]
fn test_builder_empty_input() {
    let builder = HistogramBuilder::new(10);
    let h = builder.build(&mut []);
    assert!(h.buckets.is_empty());
    assert_eq!(h.total_count, 0);
    assert_eq!(h.incremental_updates, 0);
    assert!(!h.stale);
}

#[test]
fn test_builder_single_value() {
    let builder = HistogramBuilder::new(10);
    let mut values = [42.0, 42.0, 42.0, 42.0, 42.0];
    let h = builder.build(&mut values);
    assert_eq!(h.buckets.len(), 1);
    assert_eq!(h.buckets[0].count, 5);
    assert_eq!(h.buckets[0].distinct_count, 1);
    assert!((h.buckets[0].lower_bound - 42.0).abs() < f64::EPSILON);
    assert_eq!(h.total_count, 5);
}

#[test]
fn test_builder_all_nan() {
    let builder = HistogramBuilder::new(10);
    let mut values = [f64::NAN, f64::NAN, f64::NAN];
    let h = builder.build(&mut values);
    assert!(h.buckets.is_empty());
    assert_eq!(h.total_count, 0);
}

#[test]
fn test_builder_fewer_distinct_than_buckets() {
    let builder = HistogramBuilder::new(10);
    // 3 distinct values, 10 buckets → 3 buckets (one per distinct)
    let mut values = [1.0, 1.0, 2.0, 2.0, 2.0, 3.0];
    let h = builder.build(&mut values);
    assert_eq!(h.buckets.len(), 3);
    assert_eq!(h.buckets[0].count, 2); // two 1.0s
    assert_eq!(h.buckets[0].distinct_count, 1);
    assert_eq!(h.buckets[1].count, 3); // three 2.0s
    assert_eq!(h.buckets[1].distinct_count, 1);
    assert_eq!(h.buckets[2].count, 1); // one 3.0
    assert_eq!(h.buckets[2].distinct_count, 1);
    assert_eq!(h.total_count, 6);
}

#[test]
fn test_builder_normal_distribution() {
    let builder = HistogramBuilder::new(10);
    let mut values: Vec<f64> = (0..100).map(|i| i as f64).collect();
    let h = builder.build(&mut values);
    assert_eq!(h.buckets.len(), 10);
    // Each bucket should have ~10 values
    for bucket in &h.buckets {
        assert!(bucket.count == 10);
        assert!(bucket.distinct_count == 10);
    }
    assert_eq!(h.total_count, 100);
    assert!(!h.stale);
    assert_eq!(h.incremental_updates, 0);
}

#[test]
fn test_builder_total_count_matches() {
    let builder = HistogramBuilder::new(5);
    let mut values = [1.0, f64::NAN, 2.0, 3.0, f64::NAN, 4.0, 5.0, 6.0, 7.0];
    let h = builder.build(&mut values);
    // 7 non-NaN values
    assert_eq!(h.total_count, 7);
    let bucket_sum: u64 = h.buckets.iter().map(|b| b.count).sum();
    assert_eq!(bucket_sum, h.total_count);
}

#[test]
fn test_builder_default_buckets_on_zero() {
    let builder = HistogramBuilder::new(0);
    // Should default to 64 buckets
    let mut values: Vec<f64> = (0..1000).map(|i| i as f64).collect();
    let h = builder.build(&mut values);
    // With 1000 values and 64 buckets, we get ceil(1000/64)=16 per chunk → ~63 buckets
    assert!(h.buckets.len() <= 64);
    assert!(h.buckets.len() > 0);
    assert_eq!(h.total_count, 1000);
}

#[test]
fn test_builder_mixed_nan_and_values() {
    let builder = HistogramBuilder::new(4);
    let mut values = [f64::NAN, 3.0, 1.0, f64::NAN, 2.0, 4.0];
    let h = builder.build(&mut values);
    assert_eq!(h.total_count, 4);
    // 4 distinct values, 4 buckets → one per distinct
    assert_eq!(h.buckets.len(), 4);
    // Buckets should be sorted by lower_bound
    for i in 1..h.buckets.len() {
        assert!(h.buckets[i].lower_bound >= h.buckets[i - 1].lower_bound);
    }
}

// --- Task 8.1: Histogram persistence verification tests ---

#[test]
fn test_histogram_included_in_stats_json_serialization() {
    let mut stats = CollectionStats::with_counts(1000, 0);
    let histogram = Histogram {
        buckets: vec![
            HistogramBucket {
                lower_bound: 0.0,
                upper_bound: 50.0,
                count: 400,
                distinct_count: 40,
            },
            HistogramBucket {
                lower_bound: 50.0,
                upper_bound: 100.0,
                count: 600,
                distinct_count: 55,
            },
        ],
        total_count: 1000,
        incremental_updates: 42,
        stale: true,
    };
    let mut col = ColumnStats::new("price").with_distinct_count(95);
    col.histogram = Some(histogram.clone());
    stats.column_stats.insert("price".to_string(), col.clone());
    stats.field_stats.insert("price".to_string(), col);

    // Serialize to JSON (same path as Database::analyze_collection)
    let json = serde_json::to_vec_pretty(&stats).expect("serialize stats");
    let json_str = String::from_utf8_lossy(&json);

    // Verify histogram fields are present in the JSON output
    assert!(
        json_str.contains("\"histogram\""),
        "histogram field missing"
    );
    assert!(json_str.contains("\"total_count\""), "total_count missing");
    assert!(
        json_str.contains("\"incremental_updates\""),
        "incremental_updates missing"
    );
    assert!(json_str.contains("\"stale\""), "stale missing");
    assert!(
        json_str.contains("\"distinct_count\""),
        "distinct_count missing"
    );

    // Deserialize (same path as Database::get_collection_stats)
    let restored: CollectionStats = serde_json::from_slice(&json).expect("deserialize stats");
    let restored_hist = restored
        .column_stats
        .get("price")
        .and_then(|cs| cs.histogram.as_ref())
        .expect("histogram should be present after round-trip");

    assert_eq!(restored_hist, &histogram);
    assert_eq!(restored_hist.total_count, 1000);
    assert_eq!(restored_hist.incremental_updates, 42);
    assert!(restored_hist.stale);
    assert_eq!(restored_hist.buckets.len(), 2);
    assert_eq!(restored_hist.buckets[0].distinct_count, 40);
}

#[test]
fn test_pre_histogram_stats_json_backward_compat() {
    // Simulate a stats JSON from before histogram support was added.
    // No histogram field, no distinct_count on buckets, no total_count/stale.
    let old_json = r#"{
        "total_points": 500,
        "payload_size_bytes": 1024,
        "field_stats": {
            "age": {
                "name": "age",
                "null_count": 5,
                "distinct_count": 50,
                "distinct_values": 50,
                "min_value": null,
                "max_value": null,
                "avg_size_bytes": 8
            }
        },
        "row_count": 500,
        "deleted_count": 0,
        "avg_row_size_bytes": 64,
        "total_size_bytes": 32000,
        "column_stats": {},
        "index_stats": {}
    }"#;

    let stats: CollectionStats = serde_json::from_str(old_json)
        .expect("pre-histogram JSON should deserialize with defaults");

    assert_eq!(stats.row_count, 500);
    // histogram field should default to None
    let age_stats = stats
        .field_stats
        .get("age")
        .expect("age field should exist");
    assert!(
        age_stats.histogram.is_none(),
        "histogram should default to None"
    );
}

#[test]
fn test_histogram_bucket_serde_default_distinct_count() {
    // A bucket JSON without distinct_count should deserialize with default 0.
    let bucket_json = r#"{"lower_bound": 0.0, "upper_bound": 10.0, "count": 50}"#;
    let bucket: HistogramBucket = serde_json::from_str(bucket_json)
        .expect("bucket without distinct_count should deserialize");
    assert_eq!(bucket.distinct_count, 0);
    assert_eq!(bucket.count, 50);
}

#[test]
fn test_histogram_serde_default_metadata_fields() {
    // A histogram JSON without total_count/incremental_updates/stale should use defaults.
    let hist_json = r#"{
        "buckets": [
            {"lower_bound": 0.0, "upper_bound": 10.0, "count": 100}
        ]
    }"#;
    let hist: Histogram =
        serde_json::from_str(hist_json).expect("histogram without metadata should deserialize");
    assert_eq!(hist.total_count, 0);
    assert_eq!(hist.incremental_updates, 0);
    assert!(!hist.stale);
    assert_eq!(hist.buckets.len(), 1);
    assert_eq!(hist.buckets[0].distinct_count, 0);
}

// --- WP-2A: Zero-width bucket inflation regression tests ---

#[test]
fn test_no_zero_width_buckets_with_heavy_duplicates() {
    // Regression test: duplicate-heavy data used to produce zero-width buckets
    // whose counts inflated bucket_sum(), deflating selectivity estimates.
    let builder = HistogramBuilder::new(4);
    // 20 copies of 5.0 mixed with a few distinct values → forces chunk boundaries
    // inside the 5.0 run, previously creating zero-width buckets.
    let mut values = vec![5.0; 20];
    values.extend_from_slice(&[1.0, 2.0, 3.0, 10.0, 20.0]);
    let h = builder.build(&mut values);

    // Every bucket must have non-zero width.
    for (i, bucket) in h.buckets.iter().enumerate() {
        assert!(
            bucket.upper_bound > bucket.lower_bound,
            "bucket {i} is zero-width: lower={}, upper={}",
            bucket.lower_bound,
            bucket.upper_bound,
        );
    }

    // Total count across buckets must equal number of values.
    let bucket_sum: u64 = h.buckets.iter().map(|b| b.count).sum();
    assert_eq!(bucket_sum, 25, "bucket_sum must equal total input count");
    assert_eq!(h.total_count, 25);
}

#[test]
fn test_selectivity_not_inflated_by_duplicates() {
    // Build a histogram from data with heavy duplicates and verify that
    // bucket_sum() equals total_count (no inflation from zero-width ghosts).
    let builder = HistogramBuilder::new(4);
    let mut values: Vec<f64> = vec![1.0; 50];
    values.extend_from_slice(&[2.0, 3.0, 4.0, 5.0]);
    let h = builder.build(&mut values);

    // Core invariant: bucket_sum must equal total_count — no inflation.
    let bucket_sum: u64 = h.buckets.iter().map(|b| b.count).sum();
    assert_eq!(
        bucket_sum, h.total_count,
        "bucket_sum ({bucket_sum}) != total_count ({}): zero-width inflation detected",
        h.total_count
    );

    // No zero-width buckets should remain.
    for (i, bucket) in h.buckets.iter().enumerate() {
        assert!(
            bucket.upper_bound > bucket.lower_bound,
            "bucket {i} is zero-width after merge"
        );
    }

    // Selectivity for 1.0 must be findable (not silently lost in a zero-width bucket).
    let sel = h.estimate_eq_selectivity(1.0);
    assert!(
        sel > 0.0,
        "selectivity for dominant value should be > 0, got {sel}"
    );
}

#[test]
fn test_zero_width_merge_preserves_count() {
    // Directly test the merge helper: fabricate buckets including zero-width ones
    // and verify counts are preserved after merging.
    let buckets = vec![
        HistogramBucket {
            lower_bound: 0.0,
            upper_bound: 5.0,
            count: 10,
            distinct_count: 5,
        },
        // Zero-width: lower == upper == 5.0
        HistogramBucket {
            lower_bound: 5.0,
            upper_bound: 5.0,
            count: 20,
            distinct_count: 1,
        },
        HistogramBucket {
            lower_bound: 5.0,
            upper_bound: 10.0,
            count: 15,
            distinct_count: 5,
        },
    ];
    let merged = histogram::merge_zero_width_buckets(buckets);

    // The zero-width bucket should have been absorbed.
    assert_eq!(merged.len(), 2, "zero-width bucket should be merged");

    // First bucket: original non-zero-width (count=10), unchanged.
    assert_eq!(merged[0].count, 10);
    assert_eq!(merged[0].lower_bound, 0.0);
    assert!((merged[0].upper_bound - 5.0).abs() < f64::EPSILON);

    // Second bucket: original (count=15) + absorbed zero-width (count=20) = 35.
    // The zero-width bucket is merged forward into the next non-zero-width bucket.
    assert_eq!(merged[1].count, 35);
    assert_eq!(merged[1].lower_bound, 5.0);
    assert!((merged[1].upper_bound - 10.0).abs() < f64::EPSILON);

    // Total count preserved.
    let total: u64 = merged.iter().map(|b| b.count).sum();
    assert_eq!(total, 45);
}
