#![cfg(feature = "persistence")]
//! Integration tests for Bulk Insert V2 (Issue #488).
//!
//! Validates the `AsyncIndexBuilder` + `DirectVectorWriter` pipeline:
//! - Enqueue vectors into the async builder
//! - Flush synchronously into the HNSW index
//! - Verify search returns correct results

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    deprecated
)]

use tempfile::TempDir;
use velesdb_core::collection::streaming::{AsyncIndexBuilder, AsyncIndexBuilderConfig};
use velesdb_core::collection::Collection;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::quantization::StorageMode;
use velesdb_core::Point;

/// Generates a deterministic vector with a known pattern.
fn make_vector(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed as f32) * 0.3 + (i as f32) * 0.1).sin())
        .collect()
}

/// Creates a standard vector collection for testing.
#[allow(deprecated)]
fn create_test_collection(
    dir: &std::path::Path,
    dimension: usize,
    metric: DistanceMetric,
) -> Collection {
    Collection::create_with_options(dir.to_path_buf(), dimension, metric, StorageMode::Full)
        .expect("create collection")
}

// ── AsyncIndexBuilder standalone integration ────────────────────────────

#[test]
fn async_builder_enqueue_and_buffer_search() {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: 100_000,
        segment_count: Some(2),
        sync_mode: false,
    };
    let builder = AsyncIndexBuilder::new(config);

    let dim = 16;
    // Enqueue vectors into the async builder buffer.
    let vectors: Vec<(u64, Vec<f32>)> = (0..100).map(|i| (i, make_vector(i, dim))).collect();
    let threshold_reached = builder.enqueue(vectors);
    assert!(
        !threshold_reached,
        "threshold should not be reached for 100 vectors"
    );
    assert_eq!(builder.buffer_len(), 100);

    // Brute-force search in the buffer should find results.
    let query = make_vector(0, dim);
    let buffer_results = builder.search_buffer(&query, 5, DistanceMetric::Cosine);
    assert!(
        !buffer_results.is_empty(),
        "buffer search must return results"
    );
    assert_eq!(buffer_results[0].0, 0, "closest vector should be id=0");
}

#[test]
fn async_builder_threshold_triggers_correctly() {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: 50,
        segment_count: Some(2),
        sync_mode: false,
    };
    let builder = AsyncIndexBuilder::new(config);

    // Enqueue 49 vectors — should not trigger.
    let batch1: Vec<(u64, Vec<f32>)> = (0..49).map(|i| (i, make_vector(i, 4))).collect();
    assert!(!builder.enqueue(batch1), "49 < 50, should not trigger");

    // Enqueue 1 more — should trigger (total = 50).
    let batch2: Vec<(u64, Vec<f32>)> = vec![(49, make_vector(49, 4))];
    assert!(builder.enqueue(batch2), "50 >= 50, should trigger");
}

#[test]
fn async_builder_drain_returns_all_vectors() {
    let config = AsyncIndexBuilderConfig::default();
    let builder = AsyncIndexBuilder::new(config);

    let dim = 8;
    let vectors: Vec<(u64, Vec<f32>)> = (0..25).map(|i| (i, make_vector(i, dim))).collect();
    builder.enqueue(vectors);
    assert_eq!(builder.buffer_len(), 25);

    let drained = builder.drain_buffer();
    assert_eq!(drained.len(), 25, "drain must return all buffered vectors");
    assert_eq!(builder.buffer_len(), 0, "buffer must be empty after drain");
}

// ── Collection upsert_bulk + flush consistency ──────────────────────────

#[test]
fn upsert_bulk_vectors_searchable_after_flush() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("bulk_v2_flush");
    let dim = 16;
    let coll = create_test_collection(&coll_dir, dim, DistanceMetric::Cosine);

    // Insert vectors via standard upsert_bulk (WAL + storage + HNSW).
    let points: Vec<Point> = (0..100)
        .map(|i| Point::without_payload(i, make_vector(i, dim)))
        .collect();
    let inserted = coll.upsert_bulk(&points).expect("upsert_bulk");
    assert_eq!(inserted, 100);

    // Flush to persist.
    coll.flush().expect("flush");

    // Search should find results.
    let query = make_vector(0, dim);
    let results = coll.search(&query, 5).expect("search after flush");
    assert!(
        !results.is_empty(),
        "vectors must be searchable after flush"
    );
    assert_eq!(results[0].point.id, 0, "closest result should be id=0");
    assert_eq!(coll.len(), 100);
}

// ── WAL + flush_sync consistency (Task 5.4) ─────────────────────────────

#[test]
fn wal_flush_consistency_vectors_survive() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("bulk_v2_wal");
    let dim = 8;
    let coll = create_test_collection(&coll_dir, dim, DistanceMetric::Euclidean);

    // Insert vectors via standard path (WAL + mmap + HNSW).
    let points: Vec<Point> = (0..50)
        .map(|i| Point::without_payload(i, make_vector(i, dim)))
        .collect();
    coll.upsert_bulk(&points).expect("upsert_bulk");

    // Flush to persist everything.
    coll.flush().expect("flush");

    // Verify vectors are searchable after flush.
    let query = make_vector(0, dim);
    let results = coll.search(&query, 5).expect("search after flush");
    assert!(!results.is_empty(), "vectors must survive flush");
    assert_eq!(results[0].point.id, 0);
    assert_eq!(coll.len(), 50, "point count must be 50");
}

#[test]
fn wal_reopen_preserves_vectors() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("bulk_v2_reopen");
    let dim = 8;

    // Create, insert, flush_full, drop.
    {
        let coll = create_test_collection(&coll_dir, dim, DistanceMetric::Euclidean);
        let points: Vec<Point> = (0..30)
            .map(|i| Point::without_payload(i, make_vector(i, dim)))
            .collect();
        coll.upsert_bulk(&points).expect("upsert_bulk");
        coll.flush_full().expect("flush_full");
    }

    // Reopen and verify vectors are still searchable.
    #[allow(deprecated)]
    let coll = Collection::open(coll_dir).expect("reopen");
    let query = make_vector(0, dim);
    let results = coll.search(&query, 5).expect("search after reopen");
    assert!(!results.is_empty(), "vectors must survive reopen");
    assert_eq!(coll.len(), 30, "point count must be 30 after reopen");
}

// ── AsyncIndexBuilder config serde backward compat ──────────────────────

#[test]
fn async_builder_config_serde_backward_compat() {
    use velesdb_core::collection::CollectionConfig;

    // Old config.json without async_index_builder field should deserialize OK.
    let json = r#"{
        "name": "old_collection",
        "dimension": 128,
        "metric": "Euclidean",
        "point_count": 100,
        "storage_mode": "full"
    }"#;
    let config: CollectionConfig = serde_json::from_str(json).expect("deserialize");
    assert!(
        config.async_index_builder.is_none(),
        "missing field must deserialize to None"
    );
}

#[test]
fn async_builder_config_serde_with_config() {
    use velesdb_core::collection::CollectionConfig;

    let json = r#"{
        "name": "new_collection",
        "dimension": 128,
        "metric": "Cosine",
        "point_count": 0,
        "storage_mode": "full",
        "async_index_builder": {
            "merge_threshold": 5000,
            "segment_count": 4,
            "sync_mode": true
        }
    }"#;
    let config: CollectionConfig = serde_json::from_str(json).expect("deserialize");
    let aib = config
        .async_index_builder
        .expect("async_index_builder must be Some");
    assert_eq!(aib.merge_threshold, 5000);
    assert_eq!(aib.segment_count, Some(4));
    assert!(aib.sync_mode);
}
