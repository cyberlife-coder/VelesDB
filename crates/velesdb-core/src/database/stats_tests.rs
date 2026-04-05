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
