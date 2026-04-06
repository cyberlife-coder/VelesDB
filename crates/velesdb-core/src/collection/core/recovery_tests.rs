use super::recovery;
use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use crate::point::Point;
use crate::storage::VectorStorage;
use std::path::PathBuf;

/// Creates N distinct 4-dim points with IDs 0..n.
fn make_points(n: u64) -> Vec<Point> {
    (0..n)
        .map(|i| {
            let v = f32::from(u16::try_from(i).expect("test ID fits u16"));
            Point::without_payload(i, vec![v, v + 1.0, v + 2.0, v + 3.0])
        })
        .collect()
}

// =========================================================================
// Happy path — no gap
// =========================================================================

#[test]
fn test_no_gap_returns_zero() {
    let temp = tempfile::tempdir().expect("temp dir");
    let coll =
        Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine).expect("create");

    coll.upsert(make_points(5)).expect("upsert");

    let recovered =
        recovery::recover_hnsw_gap(&coll.vector_storage, &coll.index, 4).expect("recovery");
    assert_eq!(recovered, 0);
}

#[test]
fn test_empty_collection_no_recovery() {
    let temp = tempfile::tempdir().expect("temp dir");
    let coll =
        Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine).expect("create");

    let recovered =
        recovery::recover_hnsw_gap(&coll.vector_storage, &coll.index, 4).expect("recovery");
    assert_eq!(recovered, 0);
}

// =========================================================================
// Simulated crash gap — vectors in storage but not in HNSW
// =========================================================================

#[test]
fn test_crash_gap_detected_and_recovered() {
    let temp = tempfile::tempdir().expect("temp dir");
    let coll =
        Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine).expect("create");

    coll.upsert(make_points(3)).expect("upsert");

    // Simulate crash gap: write 2 vectors to storage ONLY, bypassing HNSW.
    // Use orthogonal directions to avoid cosine ambiguity with existing points.
    {
        let mut vs = coll.vector_storage.write();
        vs.store(100, &[0.0, 0.0, 1.0, 0.0]).expect("store 100");
        vs.store(101, &[0.0, 0.0, 0.0, 1.0]).expect("store 101");
    }

    assert_eq!(coll.vector_storage.read().len(), 5);
    assert_eq!(coll.index.len(), 3);

    let recovered =
        recovery::recover_hnsw_gap(&coll.vector_storage, &coll.index, 4).expect("recovery");

    assert_eq!(recovered, 2);
    assert_eq!(coll.index.len(), 5);

    // Verify recovered vectors are searchable via HNSW.
    let results = coll.index.search(&[0.0, 0.0, 1.0, 0.0], 1);
    assert_eq!(results[0].id, 100, "recovered vector should be searchable");
}

// =========================================================================
// End-to-end: create → gap → flush → drop → reopen → verify
// =========================================================================

#[test]
fn test_gap_recovery_on_collection_reopen() {
    let temp = tempfile::tempdir().expect("temp dir");

    // Phase 1: Create, populate, and simulate gap.
    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");

        coll.upsert(make_points(3)).expect("upsert");
        coll.flush().expect("flush");

        // Simulate gap: store vectors directly without HNSW indexing.
        // Use orthogonal directions to avoid cosine ambiguity.
        {
            let mut vs = coll.vector_storage.write();
            vs.store(100, &[0.0, 0.0, 1.0, 0.0]).expect("store 100");
            vs.store(101, &[0.0, 0.0, 0.0, 1.0]).expect("store 101");
            vs.flush().expect("flush storage");
        }

        // Persist HNSW WITHOUT gap vectors (simulates crash state).
        coll.flush().expect("flush hnsw");
    }

    // Phase 2: Reopen — should auto-recover gap vectors.
    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    assert_eq!(
        reopened.index.len(),
        5,
        "HNSW should include 3 original + 2 recovered vectors"
    );

    // Verify search finds the recovered vector (orthogonal direction).
    let results = reopened.search(&[0.0, 0.0, 1.0, 0.0], 1).expect("search");
    assert!(!results.is_empty(), "search should return results");
    assert_eq!(
        results[0].point.id, 100,
        "recovered vector should be found by search"
    );
}

// =========================================================================
// Metadata-only collection — no recovery needed
// =========================================================================

#[test]
fn test_metadata_only_skips_recovery() {
    let temp = tempfile::tempdir().expect("temp dir");

    // Create metadata-only, drop, reopen — should succeed without crash.
    {
        let _coll =
            Collection::create_metadata_only(PathBuf::from(temp.path()), "meta").expect("create");
    }
    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");

    // Metadata-only has dimension 0, no vectors, no HNSW content.
    assert_eq!(reopened.config.read().dimension, 0);
}
