//! PERF1 regression tests — complete removal of the `ShardedVectors` sidecar.
//!
//! Vectors now live once, in the graph's `ContiguousVectors` (persisted in
//! `native_hnsw.vectors`). These tests pin the behaviors that the removal
//! must preserve:
//!
//! - databases written by pre-PERF1 binaries (with a `native_vectors.bin`
//!   duplicate) load with full search/rerank parity;
//! - new snapshots carry no duplicate file and keep rerank / brute-force /
//!   vacuum working after reload;
//! - brute-force never exposes tombstoned slots (deletes and upserts);
//! - cosine brute-force scores are unchanged across a save/load round-trip;
//! - vacuum on a cosine index preserves recall.

use super::persistence::{self, HnswVectorsData};
use super::{HnswIndex, SearchQuality};
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use tempfile::tempdir;

/// Deterministic pseudo-random vectors (LCG) — no external RNG dependency.
fn sample_vectors(n: u64, dimension: usize, seed: u64) -> Vec<(u64, Vec<f32>)> {
    let mut state = seed | 1;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        // Map the top 24 bits to (0, 1] — lossless in f32's mantissa.
        #[allow(clippy::cast_precision_loss)] // Reason: value < 2^24 fits f32 exactly
        let bits = (state >> 40) as f32;
        (bits + 1.0) / 16_777_216.0
    };
    (0..n)
        .map(|id| (id, (0..dimension).map(|_| next()).collect()))
        .collect()
}

/// Snapshot of live `(idx, vector)` pairs as a legacy binary's sidecar
/// would have contained them (live mappings only — deletes purged slots).
fn live_graph_pairs(index: &HnswIndex) -> Vec<(usize, Vec<f32>)> {
    let inner = index.inner.read();
    inner.with_contiguous_vectors(|vectors| {
        index
            .mappings
            .iter()
            .filter_map(|(_id, idx)| vectors.get(idx).map(|v| (idx, v.to_vec())))
            .collect()
    })
}

/// Asserts two result lists agree on ids and scores (1e-5 — fp headroom).
fn assert_results_parity(
    a: &[crate::scored_result::ScoredResult],
    b: &[crate::scored_result::ScoredResult],
) {
    assert_eq!(a.len(), b.len(), "result counts must match");
    for (ra, rb) in a.iter().zip(b) {
        assert_eq!(ra.id, rb.id, "result ids must match");
        assert!(
            (ra.score - rb.score).abs() < 1e-5,
            "score drift for id {}: {} vs {}",
            ra.id,
            ra.score,
            rb.score
        );
    }
}

// -----------------------------------------------------------------------
// Legacy-format compatibility: fixture with a native_vectors.bin duplicate
// -----------------------------------------------------------------------

/// A database written by a pre-PERF1 binary (graph + mappings + meta +
/// `native_vectors.bin` duplicate, all on the same generation) must load
/// with search + rerank parity, and the duplicate must be deleted by the
/// next save.
#[test]
fn test_legacy_fixture_with_vectors_file_loads_with_parity() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let dimension = 8;

    let index = HnswIndex::new(dimension, DistanceMetric::Cosine).unwrap();
    let data = sample_vectors(50, dimension, 42);
    for (id, v) in &data {
        index.insert(*id, v);
    }
    // Create tombstones the way a real legacy DB would have them.
    index.remove(7);
    index.insert(9, &data[11].1); // upsert: id 9 now holds vector 11's data

    index.save(path).unwrap();

    // Recreate the legacy duplicate exactly as an old binary wrote it:
    // live (idx, vector) pairs stamped with the snapshot's generation.
    let meta = persistence::load_meta(path).unwrap();
    persistence::save_vectors(
        path,
        &HnswVectorsData {
            vectors: live_graph_pairs(&index),
            generation: meta.generation,
        },
    )
    .unwrap();
    assert!(path.join("native_vectors.bin").exists());

    let loaded = HnswIndex::load(path, dimension, DistanceMetric::Cosine).unwrap();
    assert_eq!(loaded.len(), index.len());
    assert!(loaded.has_vector_storage());

    let query = &data[3].1;
    assert_results_parity(
        &index.search_brute_force(query, 10).unwrap(),
        &loaded.search_brute_force(query, 10).unwrap(),
    );
    assert_results_parity(
        &index.search_with_rerank(query, 5, 20).unwrap(),
        &loaded.search_with_rerank(query, 5, 20).unwrap(),
    );

    // The next save must delete the legacy duplicate.
    loaded.save(path).unwrap();
    assert!(!path.join("native_vectors.bin").exists());
    HnswIndex::load(path, dimension, DistanceMetric::Cosine).expect("reload after cleanup");
}

/// A legacy vectors file whose generation does not match meta proves a
/// crashed old-binary save — the load must fail with `InvalidData` exactly
/// as it did before the sidecar removal.
#[test]
fn test_legacy_vectors_file_generation_mismatch_is_invalid_data() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let dimension = 4;

    let index = HnswIndex::new(dimension, DistanceMetric::Euclidean).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0, 0.0]);
    index.save(path).unwrap();

    let meta = persistence::load_meta(path).unwrap();
    persistence::save_vectors(
        path,
        &HnswVectorsData {
            vectors: live_graph_pairs(&index),
            generation: meta.generation + 1, // stale/foreign stamp
        },
    )
    .unwrap();

    let Err(err) = HnswIndex::load(path, dimension, DistanceMetric::Euclidean) else {
        panic!("generation mismatch must fail the load")
    };
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("vectors generation"),
        "error should mention the vectors generation, got: {err}"
    );
}

// -----------------------------------------------------------------------
// New snapshot format
// -----------------------------------------------------------------------

/// A snapshot written by the current binary has no `native_vectors.bin`,
/// carries a consistent generation, and reloads with rerank, brute-force,
/// and vacuum all functional.
#[test]
fn test_new_snapshot_reload_full_functionality() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let dimension = 8;

    let index = HnswIndex::new(dimension, DistanceMetric::Cosine).unwrap();
    let data = sample_vectors(60, dimension, 7);
    for (id, v) in &data {
        index.insert(*id, v);
    }
    index.save(path).unwrap();

    assert!(!path.join("native_vectors.bin").exists());
    let meta = persistence::load_meta(path).unwrap();
    let mappings = persistence::load_mappings(path).unwrap();
    let graph_gen = persistence::load_graph_generation(path).unwrap();
    assert_eq!(meta.generation, mappings.generation);
    assert_eq!(meta.generation, graph_gen);

    let loaded = HnswIndex::load(path, dimension, DistanceMetric::Cosine).unwrap();
    assert!(loaded.has_vector_storage());

    let query = &data[5].1;
    let brute = loaded.search_brute_force(query, 5).unwrap();
    assert_eq!(brute[0].id, 5, "brute-force must find the exact vector");

    let reranked = loaded.search_with_rerank(query, 5, 20).unwrap();
    assert_eq!(reranked[0].id, 5, "rerank must find the exact vector");

    assert_eq!(loaded.vacuum(), Ok(60), "vacuum must work after reload");
    let post_vacuum = loaded.search_brute_force(query, 5).unwrap();
    assert_eq!(post_vacuum[0].id, 5, "vacuumed index must stay searchable");
}

/// Cosine brute-force scores must be identical (1e-5 fp headroom) before a
/// save and after the reload — the graph store is the same data the old
/// sidecar duplicated.
#[test]
fn test_cosine_brute_force_score_parity_across_roundtrip() {
    let dir = tempdir().unwrap();
    let dimension = 16;

    let index = HnswIndex::new(dimension, DistanceMetric::Cosine).unwrap();
    let data = sample_vectors(40, dimension, 1234);
    for (id, v) in &data {
        index.insert(*id, v);
    }

    let query = &data[20].1;
    let before = index.brute_force_search_parallel(query, 40).unwrap();

    index.save(dir.path()).unwrap();
    let loaded = HnswIndex::load(dir.path(), dimension, DistanceMetric::Cosine).unwrap();
    let after = loaded.brute_force_search_parallel(query, 40).unwrap();

    assert_results_parity(&before, &after);
    // Sanity: exact self-match at the top with cosine similarity ~1.
    assert_eq!(after[0].id, 20);
    assert!((after[0].score - 1.0).abs() < 1e-5);
}

// -----------------------------------------------------------------------
// Tombstones: deletes and upserts must never leak through brute-force
// -----------------------------------------------------------------------

#[test]
fn test_brute_force_hides_deleted_tombstones() {
    let dimension = 4;
    let index = HnswIndex::new(dimension, DistanceMetric::Cosine).unwrap();
    let data = sample_vectors(10, dimension, 99);
    for (id, v) in &data {
        index.insert(*id, v);
    }

    assert!(index.remove(3));

    // Query with the deleted vector itself: its graph slot still holds the
    // data, but the missing mapping must filter it out.
    let results = index.brute_force_search_parallel(&data[3].1, 10).unwrap();
    assert_eq!(results.len(), 9);
    assert!(
        !results.iter().any(|r| r.id == 3),
        "deleted id must not appear in brute-force results"
    );
}

#[test]
fn test_brute_force_upsert_does_not_return_old_slot() {
    let dimension = 4;
    let index = HnswIndex::new(dimension, DistanceMetric::Cosine).unwrap();

    let old = [1.0_f32, 0.0, 0.0, 0.0];
    let new = [0.0_f32, 1.0, 0.0, 0.0];
    index.insert(1, &old);
    index.insert(2, &[0.0, 0.0, 1.0, 0.0]);
    index.insert(1, &new); // upsert: old slot becomes a tombstone

    let results = index.brute_force_search_parallel(&old, 10).unwrap();

    // No duplicate ids, and id 1 must score as the NEW vector (orthogonal
    // to the old one), proving the old slot is not being read.
    let id1: Vec<_> = results.iter().filter(|r| r.id == 1).collect();
    assert_eq!(id1.len(), 1, "id 1 must appear exactly once");
    assert!(
        id1[0].score.abs() < 1e-5,
        "id 1 must score as its new (orthogonal) vector, got {}",
        id1[0].score
    );
}

// -----------------------------------------------------------------------
// Vacuum on a cosine index preserves recall
// -----------------------------------------------------------------------

#[test]
fn test_vacuum_cosine_preserves_recall() {
    let dimension = 16;
    let index = HnswIndex::new(dimension, DistanceMetric::Cosine).unwrap();
    let data = sample_vectors(200, dimension, 2024);
    for (id, v) in &data {
        index.insert(*id, v);
    }
    // Delete a block to create tombstones worth vacuuming.
    for id in 0..50u64 {
        index.remove(id);
    }

    let vacuumed = index.vacuum().expect("vacuum must succeed");
    assert_eq!(vacuumed, 150);
    assert_eq!(index.len(), 150);

    // Self-recall: every surviving vector must find itself, exactly, both
    // via brute force and via the rebuilt HNSW graph.
    for (id, v) in data.iter().filter(|(id, _)| *id >= 50).step_by(10) {
        let brute = index.search_brute_force(v, 1).unwrap();
        assert_eq!(brute[0].id, *id, "brute-force self-recall lost for {id}");
        assert!((brute[0].score - 1.0).abs() < 1e-5);

        let hnsw = index
            .search_with_quality(v, 1, SearchQuality::Accurate)
            .unwrap();
        assert_eq!(hnsw[0].id, *id, "HNSW self-recall lost for {id}");
    }
    // Deleted ids stay gone.
    let results = index.brute_force_search_parallel(&data[10].1, 200).unwrap();
    assert!(!results.iter().any(|r| r.id < 50));
    assert_eq!(results.len(), 150);
}
