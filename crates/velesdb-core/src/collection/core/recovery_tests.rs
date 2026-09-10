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
        recovery::recover_hnsw_gap(&coll.storage.vector_storage, &coll.storage.index, 4)
            .expect("recovery");
    assert_eq!(recovered, 0);
}

#[test]
fn test_empty_collection_no_recovery() {
    let temp = tempfile::tempdir().expect("temp dir");
    let coll =
        Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine).expect("create");

    let recovered =
        recovery::recover_hnsw_gap(&coll.storage.vector_storage, &coll.storage.index, 4)
            .expect("recovery");
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
        let mut vs = coll.storage.vector_storage.write();
        vs.store(100, &[0.0, 0.0, 1.0, 0.0]).expect("store 100");
        vs.store(101, &[0.0, 0.0, 0.0, 1.0]).expect("store 101");
    }

    assert_eq!(coll.storage.vector_storage.read().len(), 5);
    assert_eq!(coll.storage.index.len(), 3);

    let recovered =
        recovery::recover_hnsw_gap(&coll.storage.vector_storage, &coll.storage.index, 4)
            .expect("recovery");

    assert_eq!(recovered, 2);
    assert_eq!(coll.storage.index.len(), 5);

    // Verify recovered vectors are searchable via HNSW.
    let results = coll.storage.index.search(&[0.0, 0.0, 1.0, 0.0], 1);
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
            let mut vs = coll.storage.vector_storage.write();
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
        reopened.storage.index.len(),
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
    assert_eq!(reopened.storage.config.read().dimension, 0);
}

// =========================================================================
// Flush + reopen — gap recovery must be a no-op when index is complete
// =========================================================================

#[cfg(feature = "persistence")]
#[test]
fn test_no_gap_after_flush_and_reopen() {
    let temp = tempfile::tempdir().expect("temp dir");

    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");
        coll.upsert(make_points(8)).expect("upsert");
        coll.flush_full().expect("flush_full");
    }

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    // After a full flush + clean reopen there must be no gap.
    let recovered =
        recovery::recover_hnsw_gap(&reopened.storage.vector_storage, &reopened.storage.index, 4)
            .expect("recover");
    assert_eq!(recovered, 0, "no gap after clean flush+reopen");
    assert_eq!(
        reopened.storage.index.len(),
        8,
        "all vectors present in HNSW"
    );
}

// =========================================================================
// A save that raced the async builder — mapped, stored, never linked
// =========================================================================

/// Dispersed values (xorshift), so every point is its own nearest neighbour.
#[cfg(feature = "persistence")]
fn dispersed(id: u64) -> Vec<f32> {
    let mut state = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..8)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            #[allow(
                clippy::cast_precision_loss,
                reason = "the top 24 bits of the state are exact in an f32"
            )]
            let unit = (state >> 40) as f32 / (1u64 << 24) as f32;
            unit
        })
        .collect()
}

#[cfg(feature = "persistence")]
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("test: create snapshot dir");
    for entry in std::fs::read_dir(from).expect("test: read dir") {
        let entry = entry.expect("test: dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("test: file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("test: copy file");
        }
    }
}

/// Opening a collection re-links every mapped id the graph never linked
/// (#2246).
///
/// `upsert_bulk`'s V2 path registers each mapping and writes each vector at
/// once, and leaves the graph insert to the `AsyncIndexBuilder`. The index
/// saved here while the builder still holds every point is what a save that
/// races the bulk path persists: all mapped, none linked. Recovery skipped
/// mapped ids, so graph search could never find them again.
#[test]
#[cfg(feature = "persistence")]
fn a_mapped_id_the_graph_never_linked_is_relinked_on_open() {
    // Above the 100-point brute-force shortcut, so search walks the graph.
    const POINTS: u64 = 300;
    let temp = tempfile::tempdir().expect("temp dir");
    let live = temp.path().join("live");
    let coll = Collection::create_with_async_builder(
        live.clone(),
        8,
        DistanceMetric::Euclidean,
        crate::collection::streaming::AsyncIndexBuilderConfig {
            // Well above POINTS: the builder holds every insert.
            merge_threshold: 10_000,
            segment_count: Some(2),
        },
    )
    .expect("create");
    let points: Vec<Point> = (0..POINTS)
        .map(|id| Point::without_payload(id, dispersed(id)))
        .collect();
    assert_eq!(
        coll.upsert_bulk(&points).expect("upsert_bulk"),
        points.len()
    );
    coll.storage
        .index
        .save(&coll.storage.path)
        .expect("save with the builder still holding every point");

    // What a crash right after that save leaves on disk, twice: one copy to
    // read the saved index as it stands, one to open.
    let premise = temp.path().join("premise");
    copy_dir(&live, &premise);
    let snapshot = temp.path().join("snapshot");
    copy_dir(&live, &snapshot);

    // The premise, read without recovery. A saved empty entry point reads
    // back as slot 0 once the arena holds points, so slot 0 is exempt as the
    // entry point and every other mapped id is unlinked: 299 for pass 4 to
    // relink. Slot 0 is then reached through their links back to it.
    let saved = crate::index::HnswIndex::load(&premise, 8, DistanceMetric::Euclidean)
        .expect("load the saved index");
    let unlinked_before = saved
        .inner
        .read()
        .unlinked_nodes(saved.mappings.iter().map(|(_, idx)| idx));
    assert_eq!(
        unlinked_before.len(),
        usize::try_from(POINTS - 1).expect("test: fits a usize"),
        "the save must leave every mapped id but the entry point unlinked"
    );
    let first_slot = saved.mappings.get_idx(0).expect("id 0 is mapped");
    assert!(
        !unlinked_before.contains(&first_slot),
        "the one exempt node must be id 0's slot, the entry point"
    );
    drop(saved);

    let reopened = Collection::open(snapshot).expect("open the snapshot");

    let index = &reopened.storage.index;
    let unlinked = index
        .inner
        .read()
        .unlinked_nodes(index.mappings.iter().map(|(_, idx)| idx));
    assert!(
        unlinked.is_empty(),
        "{} mapped ids are still unlinked after open",
        unlinked.len()
    );
    // A wide ef keeps the walk from stopping on a full result set, so a miss
    // here points at linkage rather than a narrow beam; the check above is
    // the structural guarantee, this one the end-to-end confirmation.
    for id in [0, POINTS / 2, POINTS - 1] {
        let hits = reopened
            .search_with_ef(&dispersed(id), 1, 4 * POINTS as usize)
            .expect("search");
        assert_eq!(
            hits.first().map(|hit| hit.point.id),
            Some(id),
            "id {id} is mapped but graph search cannot reach it"
        );
    }
}
