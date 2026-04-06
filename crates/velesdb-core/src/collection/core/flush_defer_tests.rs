#![cfg(all(test, feature = "persistence"))]
//! Tests for Issue #423 Component 3: Deferred HNSW save in `flush()`.
//!
//! Verifies that `flush()` skips HNSW `index.save()` by default (fast path),
//! while `flush_full()` always persists the HNSW graph to disk.

#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::cast_possible_truncation
)]

use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::path::PathBuf;

/// Returns true if the HNSW graph has been persisted to disk.
///
/// Checks for `native_meta.bin` — the metadata file written by
/// `HnswIndex::save()` alongside `native_mappings.bin` and
/// `native_hnsw.graph`.
fn hnsw_saved_on_disk(path: &std::path::Path) -> bool {
    path.join("native_meta.bin").exists()
}

/// Creates N distinct 4-dim points with IDs starting from `start`.
fn make_points(start: u64, n: u64) -> Vec<Point> {
    (start..start + n)
        .map(|i| {
            let f = i as f32;
            Point::without_payload(i, vec![f, f + 1.0, f + 2.0, f + 3.0])
        })
        .collect()
}

// =========================================================================
// Component 3: flush() skips HNSW save, flush_full() includes it
// =========================================================================

/// `flush()` should NOT write hnsw.bin (deferred save).
/// Data is still recoverable via WAL + gap recovery on reopen.
#[test]
fn test_flush_skips_hnsw_save_but_data_recoverable_on_reopen() {
    let temp = tempfile::tempdir().expect("temp dir");

    // Phase 1: Create, insert, flush (not flush_full).
    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");

        coll.upsert(make_points(0, 5)).expect("upsert");

        // Use flush() — should NOT save hnsw.bin
        coll.flush().expect("flush");

        // Verify HNSW graph files do NOT exist after flush()
        // (Collection was just created, never had flush_full called)
        assert!(
            !hnsw_saved_on_disk(temp.path()),
            "flush() should NOT create HNSW files (deferred save)"
        );
    }

    // Phase 2: Reopen — gap recovery should re-insert all vectors into HNSW.
    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    assert_eq!(reopened.len(), 5, "all 5 vectors should be recoverable");

    // Verify search works after recovery.
    let results = reopened.search(&[0.0, 1.0, 2.0, 3.0], 3).expect("search");
    assert!(
        !results.is_empty(),
        "search should return results after gap recovery"
    );
}

/// `flush_full()` should ALWAYS persist HNSW graph files to disk.
#[test]
fn test_flush_full_saves_hnsw_to_disk() {
    let temp = tempfile::tempdir().expect("temp dir");

    let coll =
        Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine).expect("create");

    coll.upsert(make_points(0, 5)).expect("upsert");

    // Use flush_full() — should save HNSW graph
    coll.flush_full().expect("flush_full");

    assert!(
        hnsw_saved_on_disk(temp.path()),
        "flush_full() must create HNSW graph files"
    );

    // Verify the persisted HNSW has all 5 vectors.
    drop(coll);
    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    assert_eq!(
        reopened.index.len(),
        5,
        "HNSW should have 5 vectors after flush_full + reopen"
    );
}

/// After flush_full(), reopen should be fast (no gap recovery needed).
#[test]
fn test_flush_full_then_reopen_has_no_gap() {
    let temp = tempfile::tempdir().expect("temp dir");

    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");

        coll.upsert(make_points(0, 10)).expect("upsert");
        coll.flush_full().expect("flush_full");
    }

    // Reopen — HNSW should already have all vectors (no gap to recover).
    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    assert_eq!(
        reopened.index.len(),
        10,
        "HNSW should have all vectors (saved by flush_full)"
    );
}

/// When >10K inserts since last HNSW save, `flush()` should save HNSW
/// as a safety measure (periodic save hint).
#[test]
fn test_flush_saves_hnsw_when_insert_threshold_exceeded() {
    let temp = tempfile::tempdir().expect("temp dir");
    let coll =
        Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine).expect("create");

    // Artificially set the counter above the threshold.
    coll.inserts_since_last_hnsw_save
        .store(10_001, std::sync::atomic::Ordering::Relaxed);

    coll.upsert(make_points(0, 5)).expect("upsert");
    coll.flush().expect("flush");

    // After flush with high counter, HNSW graph files should exist.
    assert!(
        hnsw_saved_on_disk(temp.path()),
        "flush() should save HNSW when insert threshold exceeded"
    );

    // Counter should be reset after the save.
    assert_eq!(
        coll.inserts_since_last_hnsw_save
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "counter should be reset after HNSW save"
    );
}

/// `flush()` increments are tracked correctly across upsert calls.
#[test]
fn test_upsert_increments_hnsw_save_counter() {
    let temp = tempfile::tempdir().expect("temp dir");
    let coll =
        Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine).expect("create");

    coll.upsert(make_points(0, 5)).expect("upsert");
    let count = coll
        .inserts_since_last_hnsw_save
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(count, 5, "counter should be 5 after inserting 5 vectors");

    coll.upsert(make_points(10, 3)).expect("upsert2");
    let count = coll
        .inserts_since_last_hnsw_save
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(count, 8, "counter should be 8 after inserting 3 more");
}
