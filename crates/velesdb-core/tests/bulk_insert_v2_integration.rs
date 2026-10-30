#![cfg(feature = "persistence")]
//! Integration tests for Bulk Insert V2 (Issue #488).
//!
//! Validates the `AsyncIndexBuilder` + `DirectVectorWriter` pipeline:
//! - Enqueue vectors into the async builder
//! - Flush synchronously into the HNSW index
//! - Verify search returns correct results, before a flush as after it

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    deprecated
)]

use tempfile::TempDir;
use velesdb_core::collection::streaming::{AsyncIndexBuilder, AsyncIndexBuilderConfig};
use velesdb_core::distance::DistanceMetric;
use velesdb_core::quantization::StorageMode;
use velesdb_core::{Point, VectorCollection};

/// Generates a deterministic vector with a known pattern.
fn make_vector(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed as f32) * 0.3 + (i as f32) * 0.1).sin())
        .collect()
}

/// Creates a standard vector collection for testing.
fn create_test_collection(
    dir: &std::path::Path,
    dimension: usize,
    metric: DistanceMetric,
) -> VectorCollection {
    VectorCollection::create(
        dir.to_path_buf(),
        "test",
        dimension,
        metric,
        StorageMode::Full,
    )
    .expect("create collection")
}

// ── AsyncIndexBuilder standalone integration ────────────────────────────

#[test]
fn async_builder_threshold_triggers_correctly() {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: 50,
        segment_count: Some(2),
    };
    let builder = AsyncIndexBuilder::new(config);

    // Enqueue 49 vectors — should not trigger.
    let batch1: Vec<(u64, Vec<f32>)> = (0..49).map(|i| (i, make_vector(i, 4))).collect();
    assert!(!builder.enqueue(batch1), "49 < 50, should not trigger");

    // Enqueue 1 more — should trigger (total = 50).
    let batch2: Vec<(u64, Vec<f32>)> = vec![(49, make_vector(49, 4))];
    assert!(builder.enqueue(batch2), "50 >= 50, should trigger");
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
    let coll = VectorCollection::open(coll_dir).expect("reopen");
    let query = make_vector(0, dim);
    let results = coll.search(&query, 5).expect("search after reopen");
    assert!(!results.is_empty(), "vectors must survive reopen");
    assert_eq!(coll.len(), 30, "point count must be 30 after reopen");
}

// ── Crash recovery: WAL gap detection on reopen (Task 5.4) ──────────────

/// Simulates a "crash" by inserting vectors, flushing WAL only (not HNSW),
/// then reopening. The recovery path should detect the gap and re-index.
#[test]
fn crash_recovery_reindexes_gap_vectors() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("bulk_v2_crash_recovery");
    let dim = 8;

    {
        let coll = create_test_collection(&coll_dir, dim, DistanceMetric::Euclidean);

        // Insert vectors — they go to WAL + mmap + HNSW.
        let points: Vec<Point> = (0..20)
            .map(|i| Point::without_payload(i, make_vector(i, dim)))
            .collect();
        coll.upsert_bulk(&points).expect("upsert_bulk");

        // Flush WAL only (not HNSW save) to simulate partial persistence.
        // The fast flush path skips HNSW save unless threshold exceeded.
        coll.flush().expect("flush");

        // Verify search works before "crash".
        let results = coll.search(&make_vector(0, dim), 3).expect("search");
        assert!(!results.is_empty(), "search must work before crash");
    }
    // Collection dropped — simulates crash (HNSW may not be saved).

    // Reopen — recovery should detect gap and re-index.
    let coll = VectorCollection::open(coll_dir).expect("reopen after crash");
    let results = coll
        .search(&make_vector(0, dim), 5)
        .expect("search after recovery");
    assert!(
        !results.is_empty(),
        "vectors must be recoverable after crash"
    );
    assert_eq!(coll.len(), 20, "all 20 vectors must be recovered");
}

/// Validates that `flush_full` persists everything (WAL + HNSW + vectors.idx),
/// so no recovery is needed on reopen.
#[test]
fn flush_full_no_recovery_needed() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("bulk_v2_flush_full");
    let dim = 8;

    {
        let coll = create_test_collection(&coll_dir, dim, DistanceMetric::Cosine);
        let points: Vec<Point> = (0..40)
            .map(|i| Point::without_payload(i, make_vector(i, dim)))
            .collect();
        coll.upsert_bulk(&points).expect("upsert_bulk");
        coll.flush_full().expect("flush_full");
    }
    let coll = VectorCollection::open(coll_dir).expect("reopen");
    let results = coll
        .search(&make_vector(0, dim), 5)
        .expect("search after flush_full reopen");
    assert!(
        !results.is_empty(),
        "vectors must be intact after flush_full + reopen"
    );
    assert_eq!(coll.len(), 40);
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

    // The legacy `sync_mode` field is retained in this fixture on purpose:
    // older `config.json` files persisted before its removal must continue
    // to deserialize. serde drops unknown fields silently.
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
}

// ── V2 wired path: DirectVectorWriter + AsyncIndexBuilder ───────────────

#[test]
fn v2_path_produces_searchable_results() {
    let tmp = TempDir::new().expect("temp dir");
    let coll_dir = tmp.path().join("v2_searchable");
    let dim = 16;

    let config = velesdb_core::collection::streaming::AsyncIndexBuilderConfig {
        merge_threshold: 50, // Low threshold to trigger flush during test
        segment_count: Some(2),
    };
    let coll =
        VectorCollection::create_with_async_builder(coll_dir, dim, DistanceMetric::Cosine, config)
            .expect("create collection with async builder");

    // Insert 100 vectors via upsert_bulk (V2 path).
    let points: Vec<Point> = (0..100)
        .map(|i| Point::without_payload(i, make_vector(i, dim)))
        .collect();
    let inserted = coll.upsert_bulk(&points).expect("upsert_bulk V2");
    assert_eq!(inserted, 100);

    // Flush to persist.
    coll.flush().expect("flush");

    // Search should find results — verifies V2 path produces searchable vectors.
    let query = make_vector(0, dim);
    let results = coll.search(&query, 5).expect("search after V2 insert");
    assert!(
        !results.is_empty(),
        "V2 path must produce searchable vectors"
    );
    assert_eq!(coll.len(), 100);
}

#[test]
#[allow(clippy::cast_sign_loss)]
fn v2_path_maintains_recall() {
    let tmp = TempDir::new().expect("temp dir");
    let dim = 16;
    let n = 200;
    let k = 10;

    // Create collection with V2 path.
    let v2_dir = tmp.path().join("v2_recall");
    let config = velesdb_core::collection::streaming::AsyncIndexBuilderConfig {
        merge_threshold: 50,
        segment_count: Some(2),
    };
    let v2_coll =
        VectorCollection::create_with_async_builder(v2_dir, dim, DistanceMetric::Cosine, config)
            .expect("create V2 collection");

    // Create standard collection for comparison.
    let std_dir = tmp.path().join("std_recall");
    let std_coll = create_test_collection(&std_dir, dim, DistanceMetric::Cosine);

    // Insert same vectors into both.
    let points: Vec<Point> = (0..n)
        .map(|i| Point::without_payload(i as u64, make_vector(i as u64, dim)))
        .collect();
    v2_coll.upsert_bulk(&points).expect("V2 upsert");
    std_coll.upsert_bulk(&points).expect("std upsert");

    v2_coll.flush().expect("V2 flush");
    std_coll.flush().expect("std flush");

    // Compare search results — V2 should have comparable recall.
    let query = make_vector(0, dim);
    let v2_results = v2_coll.search(&query, k).expect("V2 search");
    let std_results = std_coll.search(&query, k).expect("std search");

    assert!(!v2_results.is_empty(), "V2 must return results");
    assert!(!std_results.is_empty(), "std must return results");

    // Both should find the exact match (id=0) as the closest.
    assert_eq!(v2_results[0].point.id, 0, "V2 closest must be id=0");
    assert_eq!(std_results[0].point.id, 0, "std closest must be id=0");

    // Compute recall: fraction of std top-k that appear in V2 top-k.
    let std_ids: std::collections::HashSet<u64> = std_results.iter().map(|r| r.point.id).collect();
    let v2_ids: std::collections::HashSet<u64> = v2_results.iter().map(|r| r.point.id).collect();
    let overlap = std_ids.intersection(&v2_ids).count();
    let recall = overlap as f64 / k as f64;
    assert!(
        recall >= 0.90,
        "V2 path search must match standard path within parity floor: got {recall:.2}"
    );
}

// ── Visibility before the builder drains ────────────────────────────────

/// Points per batch: above the 100-point brute-force shortcut, so a search
/// walks the graph.
const VISIBILITY_BATCH: u64 = 300;
/// A point of the second batch, which the builder still holds.
const PENDING_TARGET: u64 = VISIBILITY_BATCH + VISIBILITY_BATCH / 2;

/// Dispersed values (xorshift), so every point is its own nearest neighbour.
fn dispersed(id: u64) -> Vec<f32> {
    let mut state = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..16)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32
        })
        .collect()
}

/// A collection whose first batch a flush linked and whose second batch the
/// builder still holds: `upsert_bulk` has returned for both. Only the linked
/// batch carries the text the hybrid test searches for, so a hit on the
/// pending target there comes from its vector.
fn collection_with_pending_batch(dir: &std::path::Path) -> VectorCollection {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: 10_000,
        segment_count: None,
    };
    let coll = VectorCollection::create_with_async_builder(
        dir.join("pending"),
        16,
        DistanceMetric::Euclidean,
        config,
    )
    .expect("create collection with async builder");
    let batch = |ids: std::ops::Range<u64>, cat: &str, title: &str| -> Vec<Point> {
        ids.map(|id| {
            let payload = serde_json::json!({ "cat": cat, "title": title });
            Point::new(id, dispersed(id), Some(payload))
        })
        .collect()
    };
    coll.upsert_bulk(&batch(0..VISIBILITY_BATCH, "linked", "zebra"))
        .expect("first upsert_bulk");
    coll.flush().expect("flush links the first batch");
    coll.upsert_bulk(&batch(
        VISIBILITY_BATCH..2 * VISIBILITY_BATCH,
        "pending",
        "okapi",
    ))
    .expect("second upsert_bulk");
    coll
}

fn ids(results: &[velesdb_core::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

#[test]
fn a_bulk_point_the_builder_holds_is_found_by_dense_search() {
    let tmp = TempDir::new().expect("temp dir");
    let coll = collection_with_pending_batch(tmp.path());
    let hits = coll
        .search(&dispersed(PENDING_TARGET), 5)
        .expect("dense search");
    assert_eq!(
        ids(&hits).first(),
        Some(&PENDING_TARGET),
        "got {:?}",
        ids(&hits)
    );
}

#[test]
fn a_bulk_point_the_builder_holds_is_found_by_filtered_search() {
    let tmp = TempDir::new().expect("temp dir");
    let coll = collection_with_pending_batch(tmp.path());
    let filter = velesdb_core::Filter::new(velesdb_core::Condition::eq("cat", "pending"));
    let hits = coll
        .search_with_filter(&dispersed(PENDING_TARGET), 5, &filter)
        .expect("filtered search");
    assert_eq!(
        ids(&hits).first(),
        Some(&PENDING_TARGET),
        "got {:?}",
        ids(&hits)
    );
}

#[test]
fn a_bulk_point_the_builder_holds_is_found_by_velesql_near() {
    let tmp = TempDir::new().expect("temp dir");
    let coll = collection_with_pending_batch(tmp.path());
    let mut params = std::collections::HashMap::new();
    params.insert(
        "v".to_string(),
        serde_json::json!(dispersed(PENDING_TARGET)),
    );
    for sql in [
        "SELECT * FROM pending WHERE vector NEAR $v LIMIT 5",
        "SELECT * FROM pending WHERE vector NEAR $v AND cat = 'pending' LIMIT 5",
    ] {
        let hits = coll.execute_query_str(sql, &params).expect("VelesQL NEAR");
        assert_eq!(
            ids(&hits).first(),
            Some(&PENDING_TARGET),
            "{sql}: got {:?}",
            ids(&hits)
        );
    }
}

#[test]
fn a_bulk_point_the_builder_holds_is_found_by_hybrid_search() {
    let tmp = TempDir::new().expect("temp dir");
    let coll = collection_with_pending_batch(tmp.path());
    let hits = coll
        .hybrid_search(&dispersed(PENDING_TARGET), "zebra", 5, Some(0.5))
        .expect("hybrid search");
    assert!(ids(&hits).contains(&PENDING_TARGET), "got {:?}", ids(&hits));
}
