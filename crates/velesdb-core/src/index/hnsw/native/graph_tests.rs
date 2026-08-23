//! Tests for `graph` module - Native HNSW graph implementation.

use super::graph::NativeHnsw;
use super::layer::NodeId;
use crate::distance::DistanceMetric;
use crate::index::hnsw::native::distance::{CachedSimdDistance, CpuDistance};

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_insert_and_search() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

    // Insert some vectors
    for i in 0..100 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    assert_eq!(hnsw.len(), 100);

    // Search
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);

    assert!(!results.is_empty());
    assert!(results.len() <= 10);
    // First result should be node 0 (closest to query)
    assert_eq!(results[0].0, 0);
}

#[test]
fn test_empty_search() {
    let engine = CpuDistance::new(DistanceMetric::Cosine);
    let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

    let query = vec![1.0, 2.0, 3.0];
    let results = hnsw.search(&query, 10, 50);

    assert!(results.is_empty());
}

// =========================================================================
// TDD Tests for Heuristic Neighbor Selection (PERF-3)
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_selection_empty_candidates() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert a single vector to have valid query
    hnsw.insert(&[0.0; 32]).expect("test");

    let candidates: Vec<(NodeId, f32)> = vec![];

    let selected = hnsw.select_neighbors(&candidates, 10);
    assert!(selected.is_empty(), "Empty candidates should return empty");
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_selection_fewer_than_max() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..5 {
        hnsw.insert(&[i as f32; 32]).expect("test");
    }

    let candidates: Vec<(NodeId, f32)> = vec![(0, 0.0), (1, 1.0), (2, 2.0)];

    let selected = hnsw.select_neighbors(&candidates, 10);
    assert_eq!(
        selected,
        vec![0, 1, 2],
        "fewer-than-max short-circuit must return all candidate IDs in input order"
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_selection_respects_max() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..20 {
        hnsw.insert(&[i as f32; 32]).expect("test");
    }

    let candidates: Vec<(NodeId, f32)> = (0..15).map(|i| (i, i as f32)).collect();

    let selected = hnsw.select_neighbors(&candidates, 5);
    assert_eq!(selected.len(), 5, "Should respect max_neighbors limit");
    // First candidate (id 0, dist 0.0) is always accepted (diversity short-circuit).
    assert!(
        selected.contains(&0),
        "closest candidate (dist 0) must be selected first"
    );
    // Every selected id must come from the actual candidate set (0..15), not a phantom id.
    assert!(
        selected.iter().all(|&id| id < 15),
        "selection must draw from real candidates"
    );
    // Selection must be duplicate-free (selected_set dedup contract).
    let unique: std::collections::HashSet<_> = selected.iter().copied().collect();
    assert_eq!(
        unique.len(),
        selected.len(),
        "selected neighbors must be unique"
    );
}

#[test]
fn test_heuristic_selection_prefers_diverse_neighbors() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert diverse vectors: one at origin, cluster around (10,0,0...), spread around (0,10,0...)
    hnsw.insert(&[0.0; 32]).expect("test"); // 0: origin

    // Cluster A: near (10, 0, 0, ...)
    let mut v1 = vec![0.0; 32];
    v1[0] = 10.0;
    hnsw.insert(&v1).expect("test"); // 1
    let mut v2 = vec![0.0; 32];
    v2[0] = 10.5;
    hnsw.insert(&v2).expect("test"); // 2
    let mut v3 = vec![0.0; 32];
    v3[0] = 10.2;
    hnsw.insert(&v3).expect("test"); // 3

    // Diverse point: near (0, 10, 0, ...)
    let mut v4 = vec![0.0; 32];
    v4[1] = 10.0;
    hnsw.insert(&v4).expect("test"); // 4

    // Candidates: all close to query in euclidean terms
    let candidates: Vec<(NodeId, f32)> = vec![
        (1, 10.0), // Cluster A
        (2, 10.5), // Cluster A (close to 1)
        (3, 10.2), // Cluster A (close to 1)
        (4, 10.0), // Diverse (perpendicular direction)
    ];

    let selected = hnsw.select_neighbors(&candidates, 2);

    // Heuristic should prefer diverse selection
    // Should include node 1 (first closest) and node 4 (diverse direction)
    assert_eq!(selected.len(), 2);
    assert!(selected.contains(&1), "Should include first closest");
    // The heuristic should prefer 4 over 2,3 because 4 is in a different direction
    assert!(
        selected.contains(&4),
        "diverse node (perpendicular direction) must be preferred over redundant cluster nodes 2/3"
    );
    assert!(
        !selected.contains(&2) && !selected.contains(&3),
        "redundant cluster nodes must be rejected by the VAMANA diversity gate"
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_heuristic_fills_quota_with_closest_if_needed() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    // Insert vectors
    for i in 0..10 {
        hnsw.insert(&[i as f32; 32]).expect("test");
    }

    let candidates: Vec<(NodeId, f32)> = (0..10).map(|i| (i, i as f32)).collect();

    let selected = hnsw.select_neighbors(&candidates, 8);

    // Should fill up to max even if heuristic rejects some
    assert_eq!(
        selected.len(),
        8,
        "Should fill quota with closest candidates"
    );
    let candidate_ids: std::collections::HashSet<NodeId> =
        candidates.iter().map(|(id, _)| *id).collect();
    assert!(
        selected.iter().all(|id| candidate_ids.contains(id)),
        "all selected must be input candidates"
    );
    assert!(
        selected.contains(&0) && selected.contains(&1),
        "the two closest must be selected"
    );
    assert!(
        !selected.contains(&9),
        "the farthest candidate (9) must be the one dropped — proves the quota is filled by closeness, not arbitrarily"
    );
    let unique: std::collections::HashSet<_> = selected.iter().collect();
    assert_eq!(
        unique.len(),
        selected.len(),
        "no duplicate neighbors after backfill"
    );
}

#[test]
fn test_recall_with_heuristic_selection() {
    // Test that heuristic selection maintains good recall

    let engine = CachedSimdDistance::new(DistanceMetric::Cosine, 128);
    let hnsw = NativeHnsw::new(engine, 32, 200, 1000);

    // Insert 500 random-ish vectors
    for i in 0..500 {
        let v: Vec<f32> = (0..128)
            .map(|j| ((i * 127 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }

    // Test recall: search should find vectors close to query
    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search(&query, 10, 100);

    assert!(!results.is_empty(), "Should find results");
    assert!(results.len() >= 5, "Should find at least 5 neighbors");

    // Results should be sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i].1 >= results[i - 1].1,
            "Results should be sorted by distance"
        );
    }
}

// =========================================================================
// Phase 3, Plan 04: Concurrent graph-level insert/search with invariants
// =========================================================================

/// Parallel insert at graph level with deterministic count + search integrity.
#[test]
fn test_graph_parallel_insert_search_integrity() {
    use std::sync::Arc;
    use std::thread;

    // The safety counters are process-global: snapshot before, compare after,
    // so tests that legitimately exercise the violation detector cannot fail
    // this one through the shared counter.
    let violations_before = super::graph::safety_counters::HNSW_COUNTERS
        .snapshot()
        .invariant_violation_total;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 500));

    // Pre-populate
    for i in 0..50 {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    let mut handles = vec![];

    // 4 inserters
    for t in 0..4_usize {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..50_usize {
                #[allow(clippy::cast_precision_loss)]
                let v: Vec<f32> = (0..32).map(|j| ((t * 1000 + i) * 32 + j) as f32).collect();
                hnsw_clone.insert(&v).expect("test");
            }
        }));
    }

    // 2 searchers asserting result quality
    for _ in 0..2 {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            for i in 0..30_usize {
                #[allow(clippy::cast_precision_loss)]
                let query: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
                let results = hnsw_clone.search(&query, 5, 50);
                // Results must always be distance-sorted
                for window in results.windows(2) {
                    assert!(
                        window[0].1 <= window[1].1,
                        "Search results must be distance-sorted"
                    );
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("No deadlock or panic");
    }

    // Deterministic: 50 pre-pop + 200 parallel = 250
    assert_eq!(hnsw.len(), 250, "All inserts must be reflected in count");

    // Safety counters check: this test must not have added violations
    let snapshot = super::graph::safety_counters::HNSW_COUNTERS.snapshot();
    assert_eq!(
        snapshot.invariant_violation_total, violations_before,
        "No lock-order violations during parallel graph operations"
    );
}

// =========================================================================
// Concurrent insertion tests (Flag 6 fix - tests PRNG thread-safety indirectly)
// =========================================================================

#[test]
fn test_concurrent_insertions() {
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 1000));

    let handles: Vec<_> = (0..4)
        .map(|thread_id| {
            let hnsw_clone = Arc::clone(&hnsw);
            thread::spawn(move || {
                for i in 0..50 {
                    #[allow(clippy::cast_precision_loss)]
                    let v: Vec<f32> = (0..32)
                        .map(|j| ((thread_id * 50 + i) * 32 + j) as f32)
                        .collect();
                    hnsw_clone.insert(&v).expect("test");
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    assert_eq!(hnsw.len(), 200, "All insertions should succeed");
}

#[test]
fn test_concurrent_insert_and_search() {
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 1000));

    for i in 0..100 {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    let handles: Vec<_> = (0..4)
        .map(|thread_id| {
            let hnsw_clone = Arc::clone(&hnsw);
            thread::spawn(move || {
                for i in 0..25 {
                    if thread_id % 2 == 0 {
                        #[allow(clippy::cast_precision_loss)]
                        let v: Vec<f32> = (0..32)
                            .map(|j| ((100 + thread_id * 25 + i) * 32 + j) as f32)
                            .collect();
                        hnsw_clone.insert(&v).expect("test");
                    } else {
                        #[allow(clippy::cast_precision_loss)]
                        let query: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
                        let results = hnsw_clone.search(&query, 5, 50);
                        assert!(!results.is_empty(), "Search should return results");
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    assert!(
        hnsw.len() >= 100,
        "Index should have at least initial vectors"
    );
}

// =========================================================================
// Lock-free CAS entry-point promotion (I3)
// =========================================================================

/// Verifies that concurrent promote_entry_point calls using CAS produce a
/// valid final state: entry_point references a node at max_layer.
#[test]
fn test_cas_promote_entry_point_concurrent() {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 2000));

    // Pre-populate so the graph has valid vectors and layers
    for i in 0..200_usize {
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test: insert should succeed");
    }

    let initial_ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(
        initial_ep,
        super::graph::NO_ENTRY_POINT,
        "test: entry point must be set after pre-population"
    );

    // Spawn threads that each attempt to promote with increasing layers.
    // Only the highest layer should win.
    let mut handles = vec![];
    for t in 0..8_usize {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            // Each thread promotes with a different layer (t+1..t+5)
            for layer in (t + 1)..=(t + 5) {
                hnsw_clone.promote_entry_point(t, layer);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("test: thread must not panic");
    }

    let final_ep = hnsw.entry_point.load(Ordering::Acquire);
    let final_max = hnsw.max_layer.load(Ordering::Relaxed);

    // The final max_layer must be the highest layer any thread promoted
    // (thread 7 promotes up to layer 12)
    assert_eq!(
        final_max, 12,
        "test: max_layer must be highest promoted layer"
    );

    // The entry point must be a valid node (not NO_ENTRY_POINT)
    assert_ne!(
        final_ep,
        super::graph::NO_ENTRY_POINT,
        "test: entry point must not be sentinel after promotions"
    );
}

/// Verifies CAS promotion from NO_ENTRY_POINT (first insert race).
#[test]
fn test_cas_promote_from_empty() {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread;

    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = Arc::new(NativeHnsw::new(engine, 16, 100, 100));

    // Promote from empty concurrently — only one should succeed as first EP
    let mut handles = vec![];
    for t in 0..4_usize {
        let hnsw_clone = Arc::clone(&hnsw);
        handles.push(thread::spawn(move || {
            hnsw_clone.promote_entry_point(t, 0);
        }));
    }

    for handle in handles {
        handle.join().expect("test: thread must not panic");
    }

    let ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(
        ep,
        super::graph::NO_ENTRY_POINT,
        "test: one thread must have set the entry point"
    );
    assert!(
        ep < 4,
        "test: entry point must be one of the promoted node IDs (0..4)"
    );
}

// ============================================================================
// GPU snapshot invalidation version counter (PR-B of #634)
// ============================================================================

/// The snapshot version must bump once per insert (single and parallel
/// paths alike) so GPU caches re-read fresh vectors after every mutation.
#[cfg(feature = "gpu")]
#[test]
fn test_gpu_snapshot_version_bumps_on_every_mutation() {
    use std::sync::atomic::Ordering;

    let engine = CachedSimdDistance::new(DistanceMetric::Cosine, 8);
    let hnsw = NativeHnsw::new_with_dimension_and_alpha(engine, 16, 100, 1000, 8, 1.2)
        .expect("test: index creation should succeed");

    let v0 = hnsw.gpu_snapshot_version.load(Ordering::Acquire);
    assert_eq!(v0, 0, "fresh index starts at version 0");

    let vec_a: Vec<f32> = (0..8).map(|i| i as f32 / 10.0).collect();
    hnsw.insert(&vec_a).expect("test: insert succeeds");
    let v1 = hnsw.gpu_snapshot_version.load(Ordering::Acquire);
    assert!(v1 > v0, "single insert must bump the snapshot version");

    // Parallel insert path (big enough batch to actually go parallel —
    // `parallel_insert` falls back to sequential under 100 entries).
    let batch: Vec<Vec<f32>> = (0..120)
        .map(|seed| (0..8).map(|j| (seed + j) as f32 / 100.0).collect())
        .collect();
    let batch_refs: Vec<(&[f32], usize)> =
        batch.iter().enumerate().map(|(i, v)| (&v[..], i)).collect();
    hnsw.parallel_insert(&batch_refs)
        .expect("test: parallel_insert succeeds");
    let v2 = hnsw.gpu_snapshot_version.load(Ordering::Acquire);
    assert!(v2 > v1, "parallel insert path must also bump the version");
}

/// `reorder_for_locality` rewrites both the vector buffer AND every
/// neighbour list in place. Devin flagged on PR-B that the pre-existing
/// reorder path never invalidated GPU caches, so any snapshot / CSR
/// built from the pre-reorder topology would silently be served after
/// reorder. This test locks in the contract: the snapshot version must
/// bump once per reorder pass that actually runs (above the threshold).
#[cfg(feature = "gpu")]
#[test]
fn test_reorder_for_locality_bumps_snapshot_version() {
    use std::sync::atomic::Ordering;

    // The reorder threshold is 1000 in reorder.rs; we need enough vectors
    // for the code path to actually execute the permutation.
    const N: usize = 1024;
    const DIM: usize = 8;

    let engine = CachedSimdDistance::new(DistanceMetric::Cosine, DIM);
    let hnsw = NativeHnsw::new_with_dimension_and_alpha(engine, 16, 100, 2048, DIM, 1.2)
        .expect("test: index creation");

    let vectors: Vec<Vec<f32>> = (0..N)
        .map(|i| (0..DIM).map(|j| (i * DIM + j) as f32 / 1000.0).collect())
        .collect();
    let refs: Vec<(&[f32], usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (&v[..], i))
        .collect();
    hnsw.parallel_insert(&refs)
        .expect("test: bulk insert for reorder");
    let before = hnsw.gpu_snapshot_version.load(Ordering::Acquire);

    hnsw.reorder_for_locality().expect("test: reorder succeeds");
    let after = hnsw.gpu_snapshot_version.load(Ordering::Acquire);

    assert!(
        after > before,
        "reorder_for_locality must bump the snapshot version \
         (before={before}, after={after})"
    );
    // Snapshot mutex must be cleared too (belt-and-suspenders).
    assert!(
        hnsw.gpu_vectors_snapshot.lock().is_none(),
        "reorder_for_locality must clear the snapshot mutex"
    );
}

/// Regression for the snapshot-cache-keyed-on-count bug: a hypothetical
/// delete-then-insert that returns to the same count must still
/// invalidate the cache. With the version counter introduced by PR-B
/// (issue #634), any call to `invalidate_gpu_caches` bumps the version —
/// so a future delete path that forgets the explicit `None`-clear is
/// still safe.
///
/// We simulate the worst-case "future delete forgets to clear" by
/// manually populating the snapshot with a stale entry keyed at the
/// **current** version (mimicking a snapshot installed just before
/// a delete), then calling `invalidate_gpu_caches` and checking that
/// the next read observes staleness via the version.
#[cfg(feature = "gpu")]
#[test]
fn test_snapshot_version_detects_staleness_without_explicit_clear() {
    use std::sync::atomic::Ordering;

    let engine = CachedSimdDistance::new(DistanceMetric::Cosine, 4);
    let hnsw = NativeHnsw::new_with_dimension_and_alpha(engine, 16, 100, 1000, 4, 1.2)
        .expect("test: index creation should succeed");

    // Drive the index past its fresh state so there is a real snapshot
    // to populate.
    let v: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4];
    hnsw.insert(&v).expect("test: insert succeeds");

    // Install a fake "stale" snapshot keyed at the current version.
    let current_version = hnsw.gpu_snapshot_version.load(Ordering::Acquire);
    let fake_arc: std::sync::Arc<[f32]> = vec![9.9_f32; 4].into();
    *hnsw.gpu_vectors_snapshot.lock() = Some((current_version, 4, fake_arc.clone()));

    // Invalidate via the helper but DO NOT touch the snapshot mutex
    // manually — this mirrors a future delete path that forgets the
    // explicit `None`-clear. The version bump inside the helper must be
    // enough to detect staleness on the next read.
    hnsw.invalidate_gpu_caches();

    let post_version = hnsw.gpu_snapshot_version.load(Ordering::Acquire);
    assert!(
        post_version > current_version,
        "invalidate_gpu_caches must bump the version"
    );

    // Belt-and-suspenders: the helper also clears the mutex. Verify both
    // safety nets are in place — either one alone is sufficient for
    // correctness, but both together fail-close.
    assert!(
        hnsw.gpu_vectors_snapshot.lock().is_none(),
        "invalidate_gpu_caches must also clear the snapshot mutex (belt-and-suspenders)"
    );
}

/// A quantized collection keeps its f32 arena on disk, a Full one does not.
///
/// The whole point of #2112: in SQ8/RaBitQ the traversal runs on codes and
/// f32 is read only to re-rank, so those pages may be evicted. In Full the
/// f32 *is* the traversal data, so mapping it would fault a page per hop.
/// This pins the split at the level a user actually configures.
#[cfg(feature = "persistence")]
#[test]
fn only_a_quantized_index_puts_its_arena_on_disk() {
    use crate::index::hnsw::{HnswIndex, HnswParams};

    fn arena_files(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .expect("read_dir")
            .flatten()
            .filter(|e| crate::index::hnsw::native::arena_home::ArenaHome::is_arena_file(&e.path()))
            .count()
    }

    for (mode, expected) in [
        (crate::StorageMode::Full, 0),
        (crate::StorageMode::SQ8, 1),
        (crate::StorageMode::RaBitQ, 1),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut params = HnswParams::auto(16);
        params.storage_mode = mode;
        let index = HnswIndex::with_params_in_dir(
            16,
            crate::DistanceMetric::Cosine,
            params,
            true,
            Some(dir.path().to_path_buf()),
        )
        .expect("index");

        assert_eq!(
            arena_files(dir.path()),
            expected,
            "{mode:?} should hold {expected} arena file(s) on disk"
        );

        // Dropping the index must take its arena file with it: the file is a
        // cache of `.vectors`, so leaving it behind is pure waste.
        drop(index);
        assert_eq!(
            arena_files(dir.path()),
            0,
            "{mode:?}: dropping the index must remove its arena file"
        );
    }
}

/// A stale arena from a killed process is swept, and nothing else is.
#[cfg(feature = "persistence")]
#[test]
fn sweeping_removes_only_arena_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let arena = dir.path().join("hnsw-99.arena");
    let vectors = dir.path().join("native_hnsw.vectors");
    std::fs::write(&arena, b"stale").expect("write arena");
    std::fs::write(&vectors, b"precious").expect("write vectors");

    crate::index::hnsw::sweep_stale_arenas(dir.path());

    assert!(!arena.exists(), "a stale arena must be swept");
    assert!(
        vectors.exists(),
        "the sweep must not touch the durable vector file"
    );
}

/// A quantized collection maps its arena, searches correctly, and cleans up.
///
/// The end-to-end claim of #2112 Phase B, at the level a user configures:
/// an SQ8 collection's f32 arena is a file, the results are the same as they
/// would be on the heap, and closing the collection leaves nothing behind.
#[cfg(feature = "persistence")]
#[test]
fn a_quantized_collection_maps_its_arena_and_cleans_up() {
    use crate::index::hnsw::{HnswIndex, HnswParams};
    use crate::index::VectorIndex;

    let dir = tempfile::tempdir().expect("tempdir");
    let dimension = 8;
    // Directions must be genuinely distinct: under Cosine a vector and any
    // positive multiple of it are the same point, so a fixture that varies
    // only the magnitude of one axis makes "nearest neighbour" ambiguous and
    // the assertion below meaningless.
    let vectors: Vec<Vec<f32>> = (0..64)
        .map(|i| {
            (0..dimension)
                .map(|d| {
                    let step = u32::try_from(i * dimension + d).expect("fits u32");
                    // Deterministic spread; no RNG, so a failure names the slot.
                    f32::from((step.wrapping_mul(2_654_435_761) >> 20) as u16) + 1.0
                })
                .collect()
        })
        .collect();

    let arena_count = || {
        std::fs::read_dir(dir.path())
            .expect("read_dir")
            .flatten()
            .filter(|e| crate::index::hnsw::native::arena_home::ArenaHome::is_arena_file(&e.path()))
            .count()
    };

    let mut params = HnswParams::auto(dimension);
    params.storage_mode = crate::StorageMode::SQ8;
    let index = HnswIndex::with_params_in_dir(
        dimension,
        crate::DistanceMetric::Cosine,
        params,
        true,
        Some(dir.path().to_path_buf()),
    )
    .expect("index");

    for (i, v) in vectors.iter().enumerate() {
        index.insert(i as u64, v);
    }
    assert_eq!(arena_count(), 1, "the f32 arena should be a file on disk");

    // Every vector must be findable as its own nearest neighbour. Reading
    // them back is what proves the mapped arena is actually serving the
    // re-rank, not merely existing.
    for (i, v) in vectors.iter().enumerate() {
        let hits = index.search(v, 1);
        assert_eq!(
            hits.first().map(|r| r.id),
            Some(i as u64),
            "vector {i} should be its own nearest neighbour"
        );
    }

    drop(index);
    assert_eq!(
        arena_count(),
        0,
        "closing the index must leave no arena file behind"
    );
}

/// A fresh quantized index does not pre-create a huge arena file.
///
/// `HnswParams::auto` defaults `max_elements` to 100 000, which at 768
/// dimensions sizes the arena at ~292 MB. The heap arena pre-allocates that
/// because growing it copies the whole block; a mapped arena grows by
/// extending the file, so pre-sizing buys nothing and would put a
/// third-of-a-gigabyte file on a device before the collection holds a single
/// vector. This pins the cap rather than leaving it to a constant nobody
/// re-reads.
#[cfg(feature = "persistence")]
#[test]
fn a_fresh_arena_does_not_pre_size_to_max_elements() {
    use crate::index::hnsw::{HnswIndex, HnswParams};

    let dir = tempfile::tempdir().expect("tempdir");
    let dimension = 768;
    let mut params = HnswParams::auto(dimension);
    params.storage_mode = crate::StorageMode::SQ8;
    assert!(
        params.max_elements >= 100_000,
        "this test is only meaningful while the default is large, got {}",
        params.max_elements
    );

    let index = HnswIndex::with_params_in_dir(
        dimension,
        crate::DistanceMetric::Cosine,
        params,
        true,
        Some(dir.path().to_path_buf()),
    )
    .expect("index");

    let bytes: u64 = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .flatten()
        .filter(|e| crate::index::hnsw::native::arena_home::ArenaHome::is_arena_file(&e.path()))
        .map(|e| e.metadata().expect("metadata").len())
        .sum();

    assert!(
        bytes < 32 * 1024 * 1024,
        "an empty index should not reserve {} MB of arena",
        bytes / 1_048_576
    );
    drop(index);
}

/// Blocks are reserved, not left as holes.
///
/// A sparse mapping finds its blocks only when a page is first written, and
/// a full filesystem then has no error to return through a store
/// instruction: the kernel raises SIGBUS and the process dies. The heap
/// arena returns a recoverable `AllocationFailed`, so the mapped one must
/// surface a full disk at setup instead. Reserved blocks are what make that
/// possible, and `allocated_size` is how you tell.
#[cfg(all(unix, feature = "persistence"))]
#[test]
fn an_arena_reserves_its_blocks_rather_than_leaving_holes() {
    use crate::perf_optimizations::ContiguousVectors;
    use std::os::unix::fs::MetadataExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("reserved.arena");
    let (dimension, capacity) = (64, 4096);
    let arena = ContiguousVectors::new_file_backed(&path, dimension, capacity).expect("arena");

    let meta = std::fs::metadata(&path).expect("metadata");
    let allocated = meta.blocks() * 512;
    assert!(
        allocated >= meta.len(),
        "arena must have blocks reserved for its whole length: {allocated} allocated \
         for {} apparent bytes — a hole here is a SIGBUS on a full disk",
        meta.len()
    );
    drop(arena);
}
