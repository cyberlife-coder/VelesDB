#![cfg(feature = "persistence")]
//! E2E tests for WAL crash recovery.
//!
//! Verifies data survives non-clean shutdown via WAL replay.
//!
//! The key mechanism: `Database` has no `Drop` impl (no auto-flush).
//! When the `Database` is dropped without calling `flush_all()`, data
//! written to the WAL but not persisted to mmap/HNSW is recovered on
//! the next `Database::open()` via:
//!
//! 1. `MmapStorage::replay_wal()` — recovers vectors from the WAL
//! 2. `LogPayloadStorage` — recovers payloads from the append-only log
//! 3. `recovery::run_crash_recovery()` — re-indexes gap vectors into HNSW
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

use serde_json::json;
use std::collections::HashSet;
use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generates a deterministic, normalized vector for a given seed and dimension.
fn make_vector(seed: u64, dim: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dim)
        .map(|i| ((seed as f32) * 0.37 + (i as f32) * 0.13).sin())
        .collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Builds `count` points starting at `start_id` with payloads.
fn make_points(start_id: u64, count: u64, dim: usize) -> Vec<Point> {
    (start_id..start_id + count)
        .map(|id| {
            Point::new(
                id,
                make_vector(id, dim),
                Some(json!({ "label": format!("point-{id}") })),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Test 1: Unflushed points survive reopen
// ---------------------------------------------------------------------------

#[test]
fn test_wal_recovery_unflushed_points_survive_reopen() {
    // GIVEN: a Database on a TempDir
    let dir = TempDir::new().expect("test: create temp dir");
    let dir_path = dir.path().to_path_buf();

    let points = make_points(0, 50, 4);
    let expected_ids: HashSet<u64> = (0..50).collect();

    {
        let db = Database::open(&dir_path).expect("test: open db");
        db.create_vector_collection("wal_test", 4, DistanceMetric::Cosine)
            .expect("test: create collection");
        let coll = db
            .get_vector_collection("wal_test")
            .expect("test: get collection");

        // AND: upsert 50 points — data goes to WAL
        coll.upsert(points.clone()).expect("test: upsert");

        // Do NOT call flush — data is only in WAL, not fully persisted
        // Database has no Drop impl, so dropping it does not flush.
    }

    // WHEN: reopen the Database on the same dir
    let db2 = Database::open(&dir_path).expect("test: reopen db");
    let coll2 = db2
        .get_vector_collection("wal_test")
        .expect("test: get collection after reopen");

    // THEN: all 50 points are recoverable
    assert_eq!(
        coll2.len(),
        50,
        "all 50 points should be recovered from WAL"
    );

    // Verify specific point IDs and payloads via get()
    let retrieved = coll2.get(&(0..50).collect::<Vec<u64>>());
    let mut found_ids = HashSet::new();
    for (id, maybe_point) in (0u64..50).zip(&retrieved) {
        let p = maybe_point
            .as_ref()
            .unwrap_or_else(|| panic!("test: point {id} should exist after WAL recovery"));
        assert_eq!(p.id, id, "point ID mismatch");
        assert_eq!(
            p.vector.len(),
            4,
            "vector dimension should be 4 for point {id}"
        );
        // Verify payload
        let label = p
            .payload
            .as_ref()
            .and_then(|p| p.get("label"))
            .and_then(|v| v.as_str());
        assert_eq!(
            label,
            Some(format!("point-{id}")).as_deref(),
            "payload label mismatch for point {id}"
        );
        found_ids.insert(p.id);
    }
    assert_eq!(found_ids, expected_ids, "recovered IDs should match");

    // Search returns correct nearest neighbors
    let query = make_vector(0, 4);
    let results = coll2
        .search(&query, 5)
        .expect("test: search after recovery");
    assert!(!results.is_empty(), "search should return results");
    // The exact vector for ID 0 should be the top result
    assert_eq!(
        results[0].point.id, 0,
        "top search result should be the exact-match vector"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Partial flush — first batch flushed, second only in WAL
// ---------------------------------------------------------------------------

#[test]
fn test_wal_recovery_partial_flush() {
    let dir = TempDir::new().expect("test: create temp dir");
    let dir_path = dir.path().to_path_buf();

    {
        let db = Database::open(&dir_path).expect("test: open db");
        db.create_vector_collection("partial", 4, DistanceMetric::Cosine)
            .expect("test: create collection");
        let coll = db
            .get_vector_collection("partial")
            .expect("test: get collection");

        // Insert first 50 and flush (persisted to mmap + WAL)
        let batch_1 = make_points(0, 50, 4);
        coll.upsert(batch_1).expect("test: upsert batch 1");
        coll.flush().expect("test: flush batch 1");

        // Insert next 50 — NOT flushed (WAL only)
        let batch_2 = make_points(50, 50, 4);
        coll.upsert(batch_2).expect("test: upsert batch 2");

        // Drop without second flush
    }

    // WHEN: reopen
    let db2 = Database::open(&dir_path).expect("test: reopen db");
    let coll2 = db2
        .get_vector_collection("partial")
        .expect("test: get collection after reopen");

    // THEN: all 100 points are present (50 from mmap + 50 from WAL replay)
    assert_eq!(
        coll2.len(),
        100,
        "all 100 points should be present (50 mmap + 50 WAL)"
    );

    // Spot-check first batch (flushed)
    let p25 = &coll2.get(&[25])[0];
    assert!(p25.is_some(), "point 25 (flushed batch) should exist");

    // Spot-check second batch (WAL-only)
    let p75 = &coll2.get(&[75])[0];
    assert!(p75.is_some(), "point 75 (WAL-only batch) should exist");

    // Verify payload integrity across both batches
    for id in [0u64, 49, 50, 99] {
        let results = coll2.get(&[id]);
        let p = results[0]
            .as_ref()
            .unwrap_or_else(|| panic!("test: point {id} should exist"));
        let label = p
            .payload
            .as_ref()
            .and_then(|p| p.get("label"))
            .and_then(|v| v.as_str());
        assert_eq!(
            label,
            Some(format!("point-{id}")).as_deref(),
            "payload for point {id} should survive recovery"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: WAL recovery preserves search quality (recall)
// ---------------------------------------------------------------------------

#[test]
fn test_wal_recovery_preserves_search_quality() {
    let dir = TempDir::new().expect("test: create temp dir");
    let dir_path = dir.path().to_path_buf();

    let dim = 32;
    let n = 500;
    let points = make_points(0, n, dim);

    {
        let db = Database::open(&dir_path).expect("test: open db");
        db.create_vector_collection("recall_test", dim, DistanceMetric::Cosine)
            .expect("test: create collection");
        let coll = db
            .get_vector_collection("recall_test")
            .expect("test: get collection");

        coll.upsert(points).expect("test: upsert 500 points");

        // Drop without flush — WAL has data, HNSW not saved
    }

    // WHEN: reopen
    let db2 = Database::open(&dir_path).expect("test: reopen db");
    let coll2 = db2
        .get_vector_collection("recall_test")
        .expect("test: get collection after reopen");

    assert_eq!(
        coll2.len(),
        n as usize,
        "all 500 points should be recovered"
    );

    // Compute recall@10 by comparing HNSW results against brute-force ground truth
    let k: usize = 10;
    let num_queries = 20;
    let mut total_recall: f64 = 0.0;

    for query_seed in 0..num_queries {
        let query = make_vector(query_seed, dim);

        // HNSW search
        let hnsw_results = coll2
            .search(&query, k)
            .expect("test: search should succeed");
        let hnsw_ids: HashSet<u64> = hnsw_results.iter().map(|r| r.point.id).collect();

        // Brute-force ground truth: compute cosine similarity for all points
        let mut scored: Vec<(u64, f32)> = (0..n)
            .map(|id| {
                let v = make_vector(id, dim);
                let dot: f32 = query.iter().zip(&v).map(|(a, b)| a * b).sum();
                (id, dot)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let gt_ids: HashSet<u64> = scored.iter().take(k).map(|(id, _)| *id).collect();

        let overlap = hnsw_ids.intersection(&gt_ids).count();
        total_recall += overlap as f64 / k as f64;
    }

    let avg_recall = total_recall / num_queries as f64;
    assert!(
        avg_recall >= 0.80,
        "recall@{k} should be >= 0.80 after WAL recovery, got {avg_recall:.3}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Empty collection survives reopen
// ---------------------------------------------------------------------------

#[test]
fn test_wal_recovery_empty_collection() {
    let dir = TempDir::new().expect("test: create temp dir");
    let dir_path = dir.path().to_path_buf();

    {
        let db = Database::open(&dir_path).expect("test: open db");
        db.create_vector_collection("empty", 4, DistanceMetric::Cosine)
            .expect("test: create collection");
        // Insert 0 points, drop without flush
    }

    // WHEN: reopen
    let db2 = Database::open(&dir_path).expect("test: reopen db");
    let coll2 = db2
        .get_vector_collection("empty")
        .expect("test: empty collection should exist after reopen");

    // THEN: collection exists, is empty, no errors
    assert_eq!(coll2.len(), 0, "empty collection should remain empty");
    assert!(coll2.is_empty(), "is_empty should return true");

    // Search on empty collection should return empty results, not error
    let query = make_vector(0, 4);
    let results = coll2
        .search(&query, 5)
        .expect("test: search on empty collection should not error");
    assert!(
        results.is_empty(),
        "search on empty collection should return no results"
    );
}

// ---------------------------------------------------------------------------
// Test 5: WAL recovery with deletes
// ---------------------------------------------------------------------------

#[test]
fn test_wal_recovery_with_deletes() {
    let dir = TempDir::new().expect("test: create temp dir");
    let dir_path = dir.path().to_path_buf();

    let deleted_ids: Vec<u64> = vec![3, 7, 12, 15, 19];
    let surviving_ids: HashSet<u64> = (0..20).filter(|id| !deleted_ids.contains(id)).collect();

    {
        let db = Database::open(&dir_path).expect("test: open db");
        db.create_vector_collection("del_test", 4, DistanceMetric::Cosine)
            .expect("test: create collection");
        let coll = db
            .get_vector_collection("del_test")
            .expect("test: get collection");

        // Insert 20 points
        let points = make_points(0, 20, 4);
        coll.upsert(points).expect("test: upsert 20 points");

        // Delete 5 of them — without flush
        coll.delete(&deleted_ids).expect("test: delete 5 points");

        // Drop without flush — both inserts and deletes in WAL
    }

    // WHEN: reopen
    let db2 = Database::open(&dir_path).expect("test: reopen db");
    let coll2 = db2
        .get_vector_collection("del_test")
        .expect("test: get collection after reopen");

    // THEN: only 15 points remain
    assert_eq!(
        coll2.len(),
        15,
        "15 points should remain after recovering deletes from WAL"
    );

    // Deleted point IDs should not be retrievable
    for &id in &deleted_ids {
        let result = &coll2.get(&[id])[0];
        assert!(
            result.is_none(),
            "deleted point {id} should NOT be present after WAL recovery"
        );
    }

    // Surviving points should be present with correct payloads
    for &id in &surviving_ids {
        let result = &coll2.get(&[id])[0];
        let p = result
            .as_ref()
            .unwrap_or_else(|| panic!("test: surviving point {id} should exist"));
        assert_eq!(p.id, id);
        let label = p
            .payload
            .as_ref()
            .and_then(|p| p.get("label"))
            .and_then(|v| v.as_str());
        assert_eq!(
            label,
            Some(format!("point-{id}")).as_deref(),
            "payload for surviving point {id} should be intact"
        );
    }

    // Deleted IDs should NOT appear in search results
    let query = make_vector(3, 4); // query vector close to deleted point 3
    let results = coll2
        .search(&query, 15)
        .expect("test: search after recovery with deletes");
    let search_ids: HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    for &id in &deleted_ids {
        assert!(
            !search_ids.contains(&id),
            "deleted point {id} should NOT appear in search results"
        );
    }
}
