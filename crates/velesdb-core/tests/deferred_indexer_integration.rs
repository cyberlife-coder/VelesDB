#![cfg(feature = "persistence")]
//! Integration tests for the deferred indexer (US-366 Phase B.3).
//!
//! Verifies that vectors are buffered, searchable via brute-force scan,
//! and correctly merged into HNSW on flush.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    deprecated
)]

use serde_json::json;
use std::io::Write;
use tempfile::TempDir;
use velesdb_core::collection::streaming::DeferredIndexerConfig;
use velesdb_core::collection::CollectionConfig;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::quantization::StorageMode;
use velesdb_core::Point;

/// Creates a collection directory with a config.json that has deferred
/// indexing enabled, then opens it via `Collection::open`.
#[allow(deprecated)]
fn create_deferred_collection(
    dir: &std::path::Path,
    dimension: usize,
    metric: DistanceMetric,
    merge_threshold: usize,
) -> velesdb_core::collection::Collection {
    use velesdb_core::collection::Collection;

    // First create a normal collection so all storage files are initialized.
    let coll =
        Collection::create_with_options(dir.to_path_buf(), dimension, metric, StorageMode::Full)
            .expect("create collection");
    coll.flush().expect("flush after create");
    drop(coll);

    // Now patch config.json to enable deferred indexing.
    let config_path = dir.join("config.json");
    let config_data = std::fs::read_to_string(&config_path).expect("read config");
    let mut config: serde_json::Value = serde_json::from_str(&config_data).expect("parse config");
    config["deferred_indexing"] = json!({
        "enabled": true,
        "merge_threshold": merge_threshold,
        "max_buffer_age_ms": 5000
    });
    let patched = serde_json::to_string_pretty(&config).expect("serialize config");
    let mut f = std::fs::File::create(&config_path).expect("open config for write");
    f.write_all(patched.as_bytes()).expect("write config");
    f.sync_all().expect("sync config");

    // Reopen with deferred indexing enabled.
    Collection::open(dir.to_path_buf()).expect("reopen collection")
}

/// Generates a deterministic vector with a known pattern.
fn make_vector(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed as f32) * 0.3 + (i as f32) * 0.1).sin())
        .collect()
}

// ── Upsert + search via buffer ──────────────────────────────────────────

#[test]
fn deferred_upsert_is_searchable_before_merge() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("test_deferred");
    let coll = create_deferred_collection(&coll_dir, 4, DistanceMetric::Cosine, 1000);

    // Insert fewer vectors than merge_threshold so they stay in the buffer.
    let points: Vec<Point> = (0..5)
        .map(|i| Point::new(i, make_vector(i, 4), None))
        .collect();
    coll.upsert(points).expect("upsert");

    // Search should find them via deferred buffer brute-force.
    let results = coll.search(&make_vector(0, 4), 3).expect("search");
    assert!(
        !results.is_empty(),
        "deferred buffer must be searchable before merge"
    );
    // The closest vector should be id=0 (identical query).
    assert_eq!(results[0].point.id, 0);
}

// ── Upsert + flush merges into HNSW ────────────────────────────────────

#[test]
fn deferred_flush_drains_buffer_into_hnsw() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("test_deferred_flush");
    let coll = create_deferred_collection(&coll_dir, 4, DistanceMetric::Euclidean, 1000);

    let points: Vec<Point> = (0..10)
        .map(|i| Point::new(i, make_vector(i, 4), None))
        .collect();
    coll.upsert(points).expect("upsert");

    // Before flush: vectors are searchable via deferred buffer brute-force.
    let pre_flush = coll
        .search(&make_vector(0, 4), 5)
        .expect("search before flush");
    assert!(
        !pre_flush.is_empty(),
        "buffer search must work before flush"
    );

    // Flush drains the deferred buffer into HNSW.
    coll.flush().expect("flush");

    // After flush: vectors should still be searchable (now via HNSW index).
    let post_flush = coll
        .search(&make_vector(0, 4), 5)
        .expect("search after flush");
    assert!(
        !post_flush.is_empty(),
        "vectors must be in HNSW after flush"
    );
    assert_eq!(coll.len(), 10, "point count must be 10 after upsert");
}

// ── Bulk upsert through deferred path ──────────────────────────────────

#[test]
fn deferred_upsert_bulk_is_searchable() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("test_deferred_bulk");
    let coll = create_deferred_collection(&coll_dir, 4, DistanceMetric::Cosine, 1000);

    let points: Vec<Point> = (0..20)
        .map(|i| Point::new(i, make_vector(i, 4), None))
        .collect();
    let inserted = coll.upsert_bulk(&points).expect("upsert_bulk");
    assert_eq!(inserted, 20);

    let results = coll.search(&make_vector(0, 4), 5).expect("search");
    assert!(
        !results.is_empty(),
        "bulk-upserted vectors must be searchable via deferred buffer"
    );
}

// ── Merge threshold triggers automatic HNSW insertion ──────────────────

#[test]
fn deferred_merge_triggered_at_threshold() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("test_deferred_threshold");
    // Low threshold = 5 so the merge triggers within our test.
    let coll = create_deferred_collection(&coll_dir, 4, DistanceMetric::Cosine, 5);

    // Insert exactly threshold vectors — should trigger merge.
    let points: Vec<Point> = (0..5)
        .map(|i| Point::new(i, make_vector(i, 4), None))
        .collect();
    coll.upsert(points).expect("upsert");

    // Insert one more to push over threshold in upsert path.
    coll.upsert(vec![Point::new(99, make_vector(99, 4), None)])
        .expect("upsert extra");

    // After the merge, search should still find results.
    let results = coll.search(&make_vector(0, 4), 3).expect("search");
    assert!(
        !results.is_empty(),
        "results must be available after threshold-triggered merge"
    );
}

// ── Delete from deferred buffer ────────────────────────────────────────

#[test]
fn deferred_delete_removes_from_buffer() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("test_deferred_delete");
    let coll = create_deferred_collection(&coll_dir, 4, DistanceMetric::Euclidean, 1000);

    let points: Vec<Point> = (0..5)
        .map(|i| Point::new(i, make_vector(i, 4), None))
        .collect();
    coll.upsert(points).expect("upsert");

    // Delete id=0 while it is still in the deferred buffer.
    coll.delete(&[0]).expect("delete");

    let results = coll.search(&make_vector(0, 4), 10).expect("search");
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        !ids.contains(&0),
        "deleted ID 0 must not appear in deferred search results"
    );
}

// ── Config serde round-trip ────────────────────────────────────────────

#[test]
fn deferred_config_serde_backward_compat() {
    // Old config.json without deferred_indexing field should deserialize OK.
    let json = r#"{
        "name": "old_collection",
        "dimension": 128,
        "metric": "Euclidean",
        "point_count": 100,
        "storage_mode": "full"
    }"#;
    let config: CollectionConfig = serde_json::from_str(json).expect("deserialize");
    assert!(
        config.deferred_indexing.is_none(),
        "missing field must deserialize to None"
    );
}

#[test]
fn deferred_config_serde_enabled() {
    let json = r#"{
        "name": "new_collection",
        "dimension": 128,
        "metric": "Cosine",
        "point_count": 0,
        "storage_mode": "full",
        "deferred_indexing": {
            "enabled": true,
            "merge_threshold": 512,
            "max_buffer_age_ms": 3000
        }
    }"#;
    let config: CollectionConfig = serde_json::from_str(json).expect("deserialize");
    let di = config
        .deferred_indexing
        .expect("deferred_indexing must be Some");
    assert!(di.enabled);
    assert_eq!(di.merge_threshold, 512);
    assert_eq!(di.max_buffer_age_ms, 3000);
}

#[test]
fn deferred_config_default_is_disabled() {
    let config = DeferredIndexerConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.merge_threshold, 1024);
}
