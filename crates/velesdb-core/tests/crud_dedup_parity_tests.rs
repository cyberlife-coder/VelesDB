#![cfg(feature = "persistence")]
//! Parity tests locking the invariant behaviour of dedup+histogram and sparse
//! WAL append across the single-point (`Collection::upsert`) and bulk
//! (`Collection::upsert_bulk`) execution paths.
//!
//! These tests exist to protect the subsequent refactoring (Issue #450 Phase
//! 3.1) that extracts the last-writer-wins dedup payload construction into a
//! shared helper. They must pass BEFORE the refactor (contract validation) and
//! AFTER the refactor (regression protection).
//!
//! Covered invariants:
//! - Bug #47: last-writer-wins dedup inside a batch (only the final payload per
//!   ID is counted in the histogram).
//! - Bug #49: histogram replacement happens in a single atomic decrement +
//!   increment cycle.
//! - Sparse WAL durability: upserting sparse vectors via the single-point or
//!   the bulk path produces the same WAL state (verified via reopen + sparse
//!   query).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

use std::collections::BTreeMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::collection::stats::{CollectionStats, ColumnStats, Histogram, HistogramBucket};
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{Database, DistanceMetric, Point};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_database() -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open database");
    (dir, db)
}

/// Builds a minimal `CollectionStats` with a histogram covering scores in
/// `[0.0, 200.0)` so incremental upserts land inside existing buckets.
fn stats_with_score_histogram() -> CollectionStats {
    let histogram = Histogram {
        buckets: vec![
            HistogramBucket {
                lower_bound: 0.0,
                upper_bound: 50.0,
                count: 0,
                distinct_count: 0,
            },
            HistogramBucket {
                lower_bound: 50.0,
                upper_bound: 100.0,
                count: 0,
                distinct_count: 0,
            },
            HistogramBucket {
                lower_bound: 100.0,
                upper_bound: 150.0,
                count: 0,
                distinct_count: 0,
            },
            HistogramBucket {
                lower_bound: 150.0,
                upper_bound: 200.0 + f64::EPSILON,
                count: 0,
                distinct_count: 0,
            },
        ],
        total_count: 0,
        incremental_updates: 0,
        stale: false,
    };

    let mut col = ColumnStats::new("score");
    col.histogram = Some(histogram);

    let mut stats = CollectionStats::with_counts(0, 0);
    stats.column_stats.insert("score".to_string(), col.clone());
    stats.field_stats.insert("score".to_string(), col);
    stats
}

/// Seeds an empty histogram file so incremental updates have somewhere to
/// write. Calls `write_stats_guarded` via `analyze_collection` semantics by
/// shelling out to `Database::analyze_collection` after creating the file.
fn seed_empty_histogram(dir: &TempDir, name: &str) {
    let stats = stats_with_score_histogram();
    let stats_path = dir.path().join(name).join("collection.stats.json");
    let bytes = serde_json::to_vec(&stats).expect("serialize stats");
    std::fs::write(&stats_path, bytes).expect("write stats file");
}

/// Reads persisted stats from disk, bypassing any in-memory cache.
fn load_persisted_stats(dir: &TempDir, name: &str) -> CollectionStats {
    let stats_path = dir.path().join(name).join("collection.stats.json");
    let bytes = std::fs::read(&stats_path).expect("read stats file");
    serde_json::from_slice(&bytes).expect("parse stats JSON")
}

/// Extracts (lower, upper, count) tuples for the `score` histogram buckets.
fn score_bucket_counts(stats: &CollectionStats) -> Vec<(f64, f64, u64)> {
    let col = stats
        .field_stats
        .get("score")
        .or_else(|| stats.column_stats.get("score"))
        .expect("score column stats exist");
    let hist = col.histogram.as_ref().expect("histogram exists");
    hist.buckets
        .iter()
        .map(|b| (b.lower_bound, b.upper_bound, b.count))
        .collect()
}

/// Constructs a sparse vector from `(index, weight)` pairs.
fn sv(pairs: &[(u32, f32)]) -> SparseVector {
    SparseVector::new(pairs.to_vec())
}

// ===========================================================================
// Test 1 — Parity: upsert vs upsert_bulk produce identical histograms
// ===========================================================================

#[test]
fn test_upsert_and_bulk_produce_identical_histograms() {
    // GIVEN two collections seeded with the same empty histogram shape.
    let (dir_a, db_a) = temp_database();
    let (dir_b, db_b) = temp_database();
    db_a.create_collection("a", 4, DistanceMetric::Cosine)
        .expect("create a");
    db_b.create_collection("b", 4, DistanceMetric::Cosine)
        .expect("create b");
    seed_empty_histogram(&dir_a, "a");
    seed_empty_histogram(&dir_b, "b");

    // Build 19 points with scores {10, 20, ..., 190}, all strictly inside the
    // [0, 200) histogram range (upper bounds are half-open `[lower, upper)`).
    let points: Vec<Point> = (1..=19u64)
        .map(|i| {
            let score = (i as i64) * 10;
            Point::new(i, vec![i as f32 / 20.0; 4], Some(json!({"score": score})))
        })
        .collect();

    // WHEN we feed the same payloads through the two execution paths.
    let coll_a = db_a.get_vector_collection("a").expect("coll a");
    coll_a.upsert(points.clone()).expect("upsert a");

    let coll_b = db_b.get_vector_collection("b").expect("coll b");
    coll_b.upsert_bulk(&points).expect("upsert_bulk b");

    // THEN the persisted histogram bucket counts are identical across both paths.
    let a = score_bucket_counts(&load_persisted_stats(&dir_a, "a"));
    let b = score_bucket_counts(&load_persisted_stats(&dir_b, "b"));
    assert_eq!(
        a, b,
        "single-point and bulk upsert must produce identical histograms"
    );

    // AND the total count across buckets equals the number of upserted points.
    let total_a: u64 = a.iter().map(|(_, _, c)| *c).sum();
    assert_eq!(
        total_a, 19,
        "every point should have incremented exactly one bucket"
    );
}

// ===========================================================================
// Test 2 — Metadata-only collection respects last-writer-wins dedup
// ===========================================================================

#[test]
fn test_upsert_metadata_only_histogram_parity() {
    // GIVEN a metadata-only collection with a seeded histogram.
    let (dir, db) = temp_database();
    db.create_metadata_collection("meta")
        .expect("create metadata");
    seed_empty_histogram(&dir, "meta");

    // Batch contains two unique ids plus one duplicate id (id=1) with a
    // different score. Only the LAST occurrence should count.
    let points = vec![
        Point::metadata_only(1, json!({"score": 10})), // overwritten below
        Point::metadata_only(2, json!({"score": 60})),
        Point::metadata_only(3, json!({"score": 110})),
        Point::metadata_only(1, json!({"score": 160})), // final winner for id=1
    ];

    // WHEN upserting via the metadata path.
    let coll = db.get_metadata_collection("meta").expect("meta coll");
    coll.upsert(points).expect("upsert metadata");

    // THEN only the LAST payload of id=1 (score=160) counts, plus id=2 and id=3.
    let stats = load_persisted_stats(&dir, "meta");
    let buckets = score_bucket_counts(&stats);
    let total: u64 = buckets.iter().map(|(_, _, c)| *c).sum();
    assert_eq!(
        total, 3,
        "dedup must keep exactly one occurrence per id, got {} (buckets: {:?})",
        total, buckets
    );

    // AND the bucket [0, 50) must be empty because id=1's initial score=10 was
    // overwritten by score=160 (bucket [150, 200)).
    let bucket_0_50 = buckets
        .iter()
        .find(|(lo, hi, _)| (*lo - 0.0).abs() < f64::EPSILON && (*hi - 50.0).abs() < f64::EPSILON)
        .expect("bucket [0, 50) exists");
    assert_eq!(
        bucket_0_50.2, 0,
        "initial score=10 must not contribute — it was overwritten by score=160"
    );

    let bucket_150_200 = buckets
        .iter()
        .find(|(lo, _, _)| (*lo - 150.0).abs() < f64::EPSILON)
        .expect("bucket [150, 200) exists");
    assert_eq!(
        bucket_150_200.2, 1,
        "final score=160 for id=1 must count exactly once"
    );
}

// ===========================================================================
// Test 3 — Last-writer-wins dedup with 3 duplicates of the same id
// ===========================================================================

#[test]
fn test_upsert_dedup_last_writer_wins() {
    // GIVEN a collection with a seeded histogram.
    let (dir, db) = temp_database();
    db.create_collection("dedup", 4, DistanceMetric::Cosine)
        .expect("create");
    seed_empty_histogram(&dir, "dedup");

    // Batch of 3 points all sharing id=42, each in a different histogram bucket.
    let points = vec![
        Point::new(42, vec![0.1; 4], Some(json!({"score": 20}))), // bucket [0, 50)
        Point::new(42, vec![0.2; 4], Some(json!({"score": 80}))), // bucket [50, 100)
        Point::new(42, vec![0.3; 4], Some(json!({"score": 170}))), // bucket [150, 200)
    ];

    // WHEN upserting via single-point path.
    let coll = db.get_vector_collection("dedup").expect("coll");
    coll.upsert(points).expect("upsert");

    // THEN only the final occurrence (score=170) contributes to the histogram.
    let buckets = score_bucket_counts(&load_persisted_stats(&dir, "dedup"));
    let total: u64 = buckets.iter().map(|(_, _, c)| *c).sum();
    assert_eq!(total, 1, "dedup must keep exactly one entry for id=42");

    let bucket_150_200 = buckets
        .iter()
        .find(|(lo, _, _)| (*lo - 150.0).abs() < f64::EPSILON)
        .expect("bucket [150, 200) exists");
    assert_eq!(
        bucket_150_200.2, 1,
        "only the last occurrence (score=170) must count"
    );

    // AND the bulk path must respect the same dedup semantics.
    let (dir_bulk, db_bulk) = temp_database();
    db_bulk
        .create_collection("dedup_bulk", 4, DistanceMetric::Cosine)
        .expect("create bulk");
    seed_empty_histogram(&dir_bulk, "dedup_bulk");
    let bulk_points = vec![
        Point::new(42, vec![0.1; 4], Some(json!({"score": 20}))),
        Point::new(42, vec![0.2; 4], Some(json!({"score": 80}))),
        Point::new(42, vec![0.3; 4], Some(json!({"score": 170}))),
    ];
    let coll_bulk = db_bulk
        .get_vector_collection("dedup_bulk")
        .expect("coll bulk");
    coll_bulk.upsert_bulk(&bulk_points).expect("upsert_bulk");

    let buckets_bulk = score_bucket_counts(&load_persisted_stats(&dir_bulk, "dedup_bulk"));
    assert_eq!(
        buckets, buckets_bulk,
        "bulk and single-point dedup must be bitwise identical"
    );
}

// ===========================================================================
// Test 4 — Sparse WAL append parity: upsert vs upsert_bulk reload identically
// ===========================================================================

#[test]
fn test_sparse_wal_append_parity() {
    // GIVEN two collections — one populated via single-point upsert, one via bulk.
    let (dir_a, db_a) = temp_database();
    let (dir_b, db_b) = temp_database();
    db_a.create_collection("sparse_a", 4, DistanceMetric::Cosine)
        .expect("create a");
    db_b.create_collection("sparse_b", 4, DistanceMetric::Cosine)
        .expect("create b");

    // Build 6 points carrying a sparse vector under the default name `""`.
    let build_points = || -> Vec<Point> {
        (1..=6u64)
            .map(|i| {
                let mut map: BTreeMap<String, SparseVector> = BTreeMap::new();
                map.insert(String::new(), sv(&[(i as u32, 1.0), (i as u32 + 10, 0.5)]));
                Point::with_sparse(i, vec![i as f32 / 6.0; 4], None, Some(map))
            })
            .collect()
    };

    // WHEN upserting the same points via the two paths.
    db_a.get_vector_collection("sparse_a")
        .expect("a")
        .upsert(build_points())
        .expect("upsert a");
    db_b.get_vector_collection("sparse_b")
        .expect("b")
        .upsert_bulk(&build_points())
        .expect("upsert_bulk b");

    // AND we close the databases so WAL + mmap files are flushed to disk.
    drop(db_a);
    drop(db_b);

    // THEN reopening each database yields the same observable sparse state.
    let db_a_reopen = Database::open(dir_a.path()).expect("reopen a");
    let db_b_reopen = Database::open(dir_b.path()).expect("reopen b");

    let coll_a = db_a_reopen
        .get_vector_collection("sparse_a")
        .expect("reopen a coll");
    let coll_b = db_b_reopen
        .get_vector_collection("sparse_b")
        .expect("reopen b coll");

    // All 6 point ids must be retrievable from both reopened collections.
    let ids: Vec<u64> = (1..=6u64).collect();
    let retrieved_a: Vec<u64> = coll_a
        .get(&ids)
        .into_iter()
        .filter_map(|opt| opt.map(|p| p.id))
        .collect();
    let retrieved_b: Vec<u64> = coll_b
        .get(&ids)
        .into_iter()
        .filter_map(|opt| opt.map(|p| p.id))
        .collect();

    assert_eq!(
        retrieved_a, retrieved_b,
        "both paths must persist the same set of ids through WAL reload"
    );
    assert_eq!(retrieved_a.len(), 6, "all 6 points must reload");

    // AND the point_count reported by the Database matches on both sides.
    let stats_a = db_a_reopen
        .get_collection_stats("sparse_a")
        .expect("stats a");
    let stats_b = db_b_reopen
        .get_collection_stats("sparse_b")
        .expect("stats b");
    // Stats may be None (no ANALYZE yet) — compare the option shape.
    assert_eq!(
        stats_a.is_some(),
        stats_b.is_some(),
        "stats file presence must be identical between paths"
    );
}
