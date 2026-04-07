//! Tests for `Database::analyze_collection` and `get_collection_stats`.

#![allow(clippy::cast_precision_loss)]

use crate::database::Database;
use crate::distance::DistanceMetric;
use crate::point::Point;
use tempfile::TempDir;

/// Helper: open a database in a temp dir.
fn temp_database() -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open database");
    (dir, db)
}

/// Helper: create a collection and insert some points.
fn setup_collection(db: &Database, name: &str, dim: usize, count: u64) {
    db.create_collection(name, dim, DistanceMetric::Cosine)
        .expect("create collection");

    let coll = db.get_vector_collection(name).expect("collection exists");
    let points: Vec<Point> = (1..=count)
        .map(|i| Point {
            id: i,
            vector: vec![i as f32; dim],
            payload: None,
            sparse_vectors: None,
        })
        .collect();
    coll.upsert(points).expect("upsert");
}

// ─────────────────────────────────────────────────────────────────────────────
// analyze_collection
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn analyze_collection_returns_valid_stats() {
    let (_dir, db) = temp_database();
    setup_collection(&db, "test_stats", 4, 10);

    let stats = db.analyze_collection("test_stats").expect("analyze");
    assert_eq!(stats.total_points, 10);
}

#[test]
fn analyze_collection_nonexistent_returns_error() {
    let (_dir, db) = temp_database();
    let result = db.analyze_collection("nonexistent");
    assert!(result.is_err());
}

#[test]
fn analyze_collection_persists_to_disk() {
    let (dir, db) = temp_database();
    setup_collection(&db, "persist_stats", 4, 5);

    db.analyze_collection("persist_stats").expect("analyze");

    // The stats file should now exist on disk
    let stats_path = dir
        .path()
        .join("persist_stats")
        .join("collection.stats.json");
    assert!(stats_path.exists(), "stats file should be persisted");
}

// ─────────────────────────────────────────────────────────────────────────────
// get_collection_stats round-trip
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn get_collection_stats_returns_none_before_analyze() {
    let (_dir, db) = temp_database();
    setup_collection(&db, "no_stats", 4, 3);

    let result = db
        .get_collection_stats("no_stats")
        .expect("should not error");
    assert!(result.is_none(), "no stats before analyze");
}

#[test]
fn get_collection_stats_returns_cached_after_analyze() {
    let (_dir, db) = temp_database();
    setup_collection(&db, "cached_stats", 4, 7);

    let original = db.analyze_collection("cached_stats").expect("analyze");

    let cached = db
        .get_collection_stats("cached_stats")
        .expect("get stats")
        .expect("should be Some");
    assert_eq!(cached.total_points, original.total_points);
}

#[test]
fn get_collection_stats_loads_from_disk() {
    let dir = TempDir::new().expect("tempdir");

    // Open DB, create collection, analyze, then drop DB
    {
        let db = Database::open(dir.path()).expect("open");
        setup_collection(&db, "disk_stats", 4, 8);
        db.analyze_collection("disk_stats").expect("analyze");
    }

    // Re-open DB -- stats should be loadable from disk
    let db2 = Database::open(dir.path()).expect("reopen");
    let loaded = db2
        .get_collection_stats("disk_stats")
        .expect("get stats")
        .expect("should load from disk");
    assert_eq!(loaded.total_points, 8);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 8.1: Histogram persistence in collection.stats.json
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn analyze_collection_persists_histograms() {
    let (dir, db) = temp_database();
    db.create_collection("hist_persist", 4, DistanceMetric::Cosine)
        .expect("create");

    let coll = db
        .get_vector_collection("hist_persist")
        .expect("collection");
    // Insert points with numeric payloads so histograms get built
    let points: Vec<Point> = (1..=100)
        .map(|i| Point {
            id: i,
            vector: vec![i as f32; 4],
            payload: Some(serde_json::json!({"score": i})),
            sparse_vectors: None,
        })
        .collect();
    coll.upsert(points).expect("upsert");

    let stats = db.analyze_collection("hist_persist").expect("analyze");

    // Verify histograms were built for the "score" column
    let has_histogram = stats
        .field_stats
        .get("score")
        .or_else(|| stats.column_stats.get("score"))
        .and_then(|cs| cs.histogram.as_ref())
        .is_some_and(|h| !h.buckets.is_empty());
    assert!(
        has_histogram,
        "histogram should be built for 'score' column"
    );

    // Verify the stats file on disk contains histogram data
    let stats_path = dir
        .path()
        .join("hist_persist")
        .join("collection.stats.json");
    let bytes = std::fs::read(&stats_path).expect("read stats file");
    let json_str = String::from_utf8_lossy(&bytes);
    assert!(
        json_str.contains("\"histogram\""),
        "stats JSON should contain histogram data"
    );
    assert!(
        json_str.contains("\"total_count\""),
        "stats JSON should contain total_count"
    );
}

#[test]
fn histogram_survives_database_reopen() {
    let dir = TempDir::new().expect("tempdir");

    // Phase 1: create, insert, analyze
    {
        let db = Database::open(dir.path()).expect("open");
        db.create_collection("hist_reopen", 4, DistanceMetric::Cosine)
            .expect("create");
        let coll = db.get_vector_collection("hist_reopen").expect("collection");
        let points: Vec<Point> = (1..=50)
            .map(|i| Point {
                id: i,
                vector: vec![i as f32; 4],
                payload: Some(serde_json::json!({"value": i * 10})),
                sparse_vectors: None,
            })
            .collect();
        coll.upsert(points).expect("upsert");
        db.analyze_collection("hist_reopen").expect("analyze");
    }

    // Phase 2: reopen and verify histograms are restored
    let db2 = Database::open(dir.path()).expect("reopen");
    let loaded = db2
        .get_collection_stats("hist_reopen")
        .expect("get stats")
        .expect("stats should load from disk");

    let hist = loaded
        .field_stats
        .get("value")
        .or_else(|| loaded.column_stats.get("value"))
        .and_then(|cs| cs.histogram.as_ref())
        .expect("histogram should be restored after reopen");

    assert!(!hist.buckets.is_empty(), "histogram should have buckets");
    assert!(hist.total_count > 0, "total_count should be positive");
    assert_eq!(
        hist.incremental_updates, 0,
        "fresh analyze has zero updates"
    );
    assert!(!hist.stale, "fresh histogram should not be stale");
}
