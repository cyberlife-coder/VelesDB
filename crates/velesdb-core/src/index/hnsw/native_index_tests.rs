#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for `native_index` module - Native HNSW index implementation.

#![allow(clippy::useless_vec)]

use super::native_index::*;
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use tempfile::tempdir;

#[test]
fn test_native_index_new() {
    let index = NativeHnswIndex::new(64, DistanceMetric::Euclidean).expect("test");
    assert_eq!(index.dimension(), 64);
    assert_eq!(index.metric(), DistanceMetric::Euclidean);
    assert!(index.is_empty());
}

#[test]
fn test_native_index_insert_search() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean).expect("test");

    for i in 0..50 {
        let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
        index.insert(i, &vec).expect("test");
    }

    assert_eq!(index.len(), 50);

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
    let results = index.search(&query, 5);

    assert!(!results.is_empty());
    assert!(results.len() <= 5);
    assert_eq!(results[0].id, 0);
}

#[test]
fn test_native_index_batch_insert() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean).expect("test");

    let items: Vec<(u64, Vec<f32>)> = (0..50).map(|i| (i, vec![i as f32 * 0.01; 32])).collect();

    index.insert_batch(&items).expect("test");

    assert_eq!(index.len(), 50);

    let query = vec![0.0_f32; 32];
    let results = index.search(&query, 1);
    assert_eq!(results.len(), 1, "batch-inserted graph must be searchable");
    assert_eq!(
        results[0].id, 0,
        "exact-match query must return batch-inserted vector 0"
    );
}

#[test]
fn test_native_index_persistence() {
    let dir = tempdir().unwrap();

    let index = NativeHnswIndex::new(32, DistanceMetric::Cosine).expect("test");
    for i in 0..30 {
        index.insert(i, &vec![i as f32 * 0.1; 32]).expect("test");
    }

    index.save(dir.path()).unwrap();

    let loaded = NativeHnswIndex::load(dir.path(), 32, DistanceMetric::Cosine).unwrap();

    assert_eq!(loaded.dimension(), 32);
    assert_eq!(loaded.metric(), DistanceMetric::Cosine);
    assert_eq!(loaded.len(), 30);

    let results = loaded.search(&vec![0.0; 32], 5);
    assert!(!results.is_empty());

    // Ensure graph-stored vectors survive reload for brute-force APIs.
    let brute_force = loaded.brute_force_search_parallel(&vec![0.0; 32], 5);
    assert_eq!(brute_force.len(), 5);
}

#[test]
fn test_native_index_save_does_not_persist_legacy_vectors_file() {
    let dir = tempdir().unwrap();

    // PERF1: no index variant writes native_vectors.bin anymore — the
    // vectors live inside the graph dump (native_hnsw.vectors).
    let index = NativeHnswIndex::new(16, DistanceMetric::Cosine).expect("test");
    for i in 0..10 {
        index.insert(i, &vec![i as f32 * 0.1; 16]).expect("test");
    }
    index.save(dir.path()).unwrap();
    assert!(!dir.path().join("native_vectors.bin").exists());
    assert!(dir.path().join("native_hnsw.vectors").exists());
}

#[test]
fn test_native_index_fast_insert_save_does_not_persist_vectors() {
    let dir = tempdir().unwrap();

    let index = NativeHnswIndex::new_fast_insert(16, DistanceMetric::Cosine).expect("test");
    for i in 0..10 {
        index.insert(i, &vec![i as f32 * 0.1; 16]).expect("test");
    }

    index.save(dir.path()).unwrap();
    assert!(!dir.path().join("native_vectors.bin").exists());

    let loaded = NativeHnswIndex::load(dir.path(), 16, DistanceMetric::Cosine).unwrap();
    assert!(!loaded.has_vector_storage());
    assert!(loaded
        .brute_force_search_parallel(&vec![0.0; 16], 5)
        .is_empty());
}

#[test]
fn test_native_index_save_removes_stale_legacy_vectors_file() {
    let dir = tempdir().unwrap();

    let index = NativeHnswIndex::new(16, DistanceMetric::Cosine).expect("test");
    index.insert(1, &vec![0.1; 16]).expect("test");
    index.save(dir.path()).unwrap();

    // Plant a legacy vectors file as an old binary would have left it.
    std::fs::write(dir.path().join("native_vectors.bin"), [0_u8; 8]).unwrap();

    index.insert(2, &vec![0.2; 16]).expect("test");
    index.save(dir.path()).unwrap();

    assert!(!dir.path().join("native_vectors.bin").exists());
    let loaded = NativeHnswIndex::load(dir.path(), 16, DistanceMetric::Cosine).unwrap();
    assert_eq!(loaded.len(), 2);
}

#[test]
fn test_native_index_delete() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean).expect("test");
    index.insert(1, &vec![0.1; 32]).expect("test");
    index.insert(2, &vec![0.2; 32]).expect("test");

    assert!(index.remove(1));

    let results = index.search(&vec![0.1; 32], 2);
    assert!(
        !results.iter().any(|r| r.id == 1),
        "removed id=1 must not appear in HNSW search results"
    );
    assert!(
        results.iter().any(|r| r.id == 2),
        "surviving id=2 must still be searchable"
    );

    assert!(!index.remove(999));
}

#[test]
fn test_native_index_vector_index_trait() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean).expect("test");

    <NativeHnswIndex as VectorIndex>::insert(&index, 1, &vec![0.1; 32]);
    assert_eq!(<NativeHnswIndex as VectorIndex>::len(&index), 1);

    let results = <NativeHnswIndex as VectorIndex>::search(&index, &vec![0.1; 32], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);
}

#[test]
fn test_native_index_brute_force_search() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean).expect("test");

    for i in 0..20u64 {
        let vec: Vec<f32> = (0..32u64).map(|j| (i * 32 + j) as f32 * 0.001).collect();
        index.insert(i, &vec).expect("test");
    }

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.001).collect();
    let results = index.brute_force_search_parallel(&query, 5);

    assert_eq!(results.len(), 5);
    assert_eq!(results[0].id, 0);
    for i in 1..results.len() {
        assert!(
            results[i].score >= results[i - 1].score,
            "Results not sorted"
        );
    }
}

#[test]
fn test_native_index_brute_force_empty() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean).expect("test");
    let query = vec![0.0; 32];
    let results = index.brute_force_search_parallel(&query, 5);
    assert!(results.is_empty());
}

#[test]
fn test_native_index_brute_force_k_larger_than_size() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean).expect("test");
    index.insert(1, &vec![0.1; 32]).expect("test");
    index.insert(2, &vec![0.2; 32]).expect("test");

    let results = index.brute_force_search_parallel(&vec![0.0; 32], 10);
    assert_eq!(results.len(), 2);
}

// -------------------------------------------------------------------------
// Upsert Semantics Tests (Issue #371 — TDD Cycle 4)
// -------------------------------------------------------------------------

#[test]
fn test_native_insert_same_id_updates_vector() {
    // Arrange: create index with exact-distance features on (default)
    let index = NativeHnswIndex::new(4, DistanceMetric::Cosine).expect("test");

    // Insert id=1 with vector A (pointing along x-axis)
    let vector_a = [1.0, 0.0, 0.0, 0.0];
    index.insert(1, &vector_a).expect("test");

    // Act: insert id=1 again with vector B (pointing along y-axis, orthogonal to A)
    let vector_b = [0.0, 1.0, 0.0, 0.0];
    index.insert(1, &vector_b).expect("test");

    // Assert 1: index length must still be 1 (not 2)
    assert_eq!(index.len(), 1, "Upsert must not create duplicate entries");

    // Assert 2: search with query=B should return id=1 with high similarity
    let results = index.search(&vector_b, 1);
    assert_eq!(results.len(), 1, "Should find exactly one result");
    assert_eq!(results[0].id, 1, "Result must be id=1");
    assert!(
        results[0].score > 0.9,
        "Similarity to updated vector B should be > 0.9, got {}",
        results[0].score,
    );
}

#[test]
fn test_native_batch_upsert_updates_existing() {
    // Arrange: create index and insert 10 vectors via batch
    let index = NativeHnswIndex::new(4, DistanceMetric::Cosine).expect("test");

    let initial: Vec<(u64, Vec<f32>)> = (0..10).map(|i| (i, vec![1.0, 0.0, 0.0, 0.0])).collect();
    index.insert_batch(&initial).expect("test");
    assert_eq!(
        index.len(),
        10,
        "Should have 10 vectors after initial batch"
    );

    // Act: update 5 vectors (ids 0..5) with a different direction
    let updates: Vec<(u64, Vec<f32>)> = (0..5).map(|i| (i, vec![0.0, 1.0, 0.0, 0.0])).collect();
    index.insert_batch(&updates).expect("test");

    // Assert 1: total count must still be 10 (not 15)
    assert_eq!(index.len(), 10, "Upsert batch must not inflate count");

    // Assert 2: searching with the updated direction should find updated vectors
    let query = [0.0, 1.0, 0.0, 0.0];
    let results = index.search(&query, 5);
    assert!(!results.is_empty(), "Search must return results");
    // The top result should be one of the updated ids (0..5) with high similarity
    assert!(
        results[0].id < 5,
        "Top result should be an updated vector (id < 5), got id={}",
        results[0].id,
    );
    assert!(
        results[0].score > 0.9,
        "Updated vector similarity should be > 0.9, got {}",
        results[0].score,
    );
}

#[test]
fn test_native_remove_cleans_up_vector_storage() {
    // Arrange: insert a vector with storage enabled
    let index = NativeHnswIndex::new(4, DistanceMetric::Cosine).expect("test");
    index.insert(1, &[1.0, 0.0, 0.0, 0.0]).expect("test");

    // Verify vector exists in brute-force (reads the graph's ContiguousVectors)
    let before = index.brute_force_search_parallel(&[1.0, 0.0, 0.0, 0.0], 1);
    assert_eq!(before.len(), 1, "Should find vector before removal");

    // Act: remove the vector
    assert!(index.remove(1), "Remove should return true for existing ID");

    // Assert: brute-force search should find nothing (vector storage cleaned up)
    let after = index.brute_force_search_parallel(&[1.0, 0.0, 0.0, 0.0], 1);
    assert!(
        after.is_empty(),
        "Brute-force should find nothing after removal, got {} results",
        after.len(),
    );
}

// =========================================================================
// Issue #396: a batch after single inserts — each id follows its slot
// =========================================================================

/// Regression test (#396): a batch after single inserts maps each id to the
/// slot the graph gave its vector.
#[test]
fn test_native_batch_after_single_insert_mapping_consistency() {
    let index = NativeHnswIndex::new(4, DistanceMetric::Euclidean).expect("test");

    // Single-insert 2 vectors: consumes graph node 0 and 1
    index.insert(100, &[1.0, 0.0, 0.0, 0.0]).expect("test");
    index.insert(101, &[0.0, 1.0, 0.0, 0.0]).expect("test");
    assert_eq!(index.len(), 2);

    // Batch-insert 4 vectors
    let batch: Vec<(u64, Vec<f32>)> = vec![
        (200, vec![0.0, 0.0, 1.0, 0.0]),
        (201, vec![0.0, 0.0, 0.0, 1.0]),
        (202, vec![0.5, 0.5, 0.0, 0.0]),
        (203, vec![0.0, 0.5, 0.5, 0.0]),
    ];
    index.insert_batch(&batch).expect("test");
    assert_eq!(index.len(), 6);

    // Every external ID must have a consistent bidirectional mapping
    for &ext_id in &[100u64, 101, 200, 201, 202, 203] {
        let idx = index.mappings.get_idx(ext_id);
        assert!(idx.is_some(), "get_idx({ext_id}) must return Some");
        let reverse = index.mappings.get_id(idx.unwrap());
        assert_eq!(
            reverse,
            Some(ext_id),
            "Reverse mapping for idx {} must be {ext_id}, got {reverse:?}",
            idx.unwrap()
        );
    }

    // Search must find the correct nearest neighbor
    let results = index.search(&[1.0, 0.0, 0.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 100);
}

/// Regression test: sidecar vectors stored at graph-assigned IDs for NativeHnswIndex.
#[test]
fn test_native_batch_insert_vector_storage_uses_assigned_ids() {
    let index = NativeHnswIndex::new(4, DistanceMetric::Euclidean).expect("test");

    // Pre-populate to create a gap
    for i in 0..3u64 {
        index.insert(i, &[i as f32, 0.0, 0.0, 0.0]).expect("test");
    }

    // Batch-insert
    let batch: Vec<(u64, Vec<f32>)> = vec![
        (10, vec![10.0, 0.0, 0.0, 0.0]),
        (11, vec![11.0, 0.0, 0.0, 0.0]),
        (12, vec![12.0, 0.0, 0.0, 0.0]),
    ];
    index.insert_batch(&batch).expect("test");

    // Brute-force search reads the graph's ContiguousVectors through the
    // mappings — if the mapping indices are wrong, brute-force results will
    // disagree with HNSW search results.
    let hnsw_results = index.search(&[10.0, 0.0, 0.0, 0.0], 1);
    let brute_results = index.brute_force_search_parallel(&[10.0, 0.0, 0.0, 0.0], 1);

    assert_eq!(hnsw_results.len(), 1);
    assert_eq!(brute_results.len(), 1);
    assert_eq!(
        hnsw_results[0].id, brute_results[0].id,
        "HNSW and brute-force must agree on nearest neighbor"
    );
    assert_eq!(hnsw_results[0].id, 10);
}

/// Regression test: batch upsert maintains mapping consistency for NativeHnswIndex.
#[test]
fn test_native_batch_upsert_mapping_consistency() {
    let index = NativeHnswIndex::new(4, DistanceMetric::Euclidean).expect("test");

    // Insert 5 vectors
    for i in 0..5u64 {
        index.insert(i, &[i as f32, 0.0, 0.0, 0.0]).expect("test");
    }
    assert_eq!(index.len(), 5);

    // Batch-upsert: update IDs 1 and 2
    let batch: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![100.0, 0.0, 0.0, 0.0]),
        (2, vec![200.0, 0.0, 0.0, 0.0]),
    ];
    index.insert_batch(&batch).expect("test");
    assert_eq!(index.len(), 5); // Still 5 (replaced, not added)

    // All IDs must have consistent bidirectional mappings
    for ext_id in 0..5u64 {
        let idx = index.mappings.get_idx(ext_id);
        assert!(idx.is_some(), "get_idx({ext_id}) must return Some");
        let reverse = index.mappings.get_id(idx.unwrap());
        assert_eq!(
            reverse,
            Some(ext_id),
            "Reverse mapping for idx {} must be {ext_id}",
            idx.unwrap()
        );
    }
}

// =============================================================================
// Alpha wiring tests
// =============================================================================

#[test]
fn test_native_index_with_params_uses_custom_alpha() {
    use super::params::HnswParams;

    // GIVEN: params with a custom alpha
    let params = HnswParams::auto(32).with_alpha(1.0);

    // WHEN: create index with those params
    let index =
        NativeHnswIndex::with_params(32, DistanceMetric::Cosine, params).expect("test: create");

    // THEN: insert and search should work (alpha affects graph structure, not API)
    for i in 0..20u64 {
        let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
        index.insert(i, &vec).expect("test: insert");
    }

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
    let results = index.search(&query, 5);
    assert!(!results.is_empty(), "search should return results");
    assert_eq!(results[0].id, 0, "nearest neighbor should be point 0");
}

#[test]
fn test_native_index_default_alpha_search_works() {
    use super::params::HnswParams;

    // GIVEN: default params (alpha = 1.2)
    let params = HnswParams::auto(32);
    assert!(
        (params.alpha - 1.2).abs() < f32::EPSILON,
        "default alpha should be 1.2"
    );

    // WHEN: create index and insert data
    let index =
        NativeHnswIndex::with_params(32, DistanceMetric::Cosine, params).expect("test: create");

    for i in 0..20u64 {
        let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
        index.insert(i, &vec).expect("test: insert");
    }

    // THEN: search works correctly
    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
    let results = index.search(&query, 5);
    assert!(!results.is_empty(), "search should return results");
    assert_eq!(
        results[0].id, 0,
        "nearest neighbor of vector[0] should be node 0 with default alpha"
    );
}

/// Two saves of one `NativeHnswIndex` into one directory never mix their
/// files (#2262).
///
/// A save rewrites its graph file in place and stamps every artefact with the
/// generation after the one it reads from the directory, so two at once write
/// one graph file at the same offsets, each from its own reading of neighbor
/// lists a writer is still changing, under one generation. `HnswIndex` takes
/// a `saving` lock of its own for exactly this; `NativeHnswIndex` is the same
/// public type over the same files, and the invariant in `docs/SOUNDNESS.md`
/// is stated of saves, not of one wrapper.
///
/// Each round inserts a batch on a thread of its own while two threads save,
/// and the directory is reloaded after every round: a reload that errors, or
/// that resolves an id to a vector that is not its own, is a torn directory.
#[test]
fn native_saves_racing_into_one_directory_reload_consistent() {
    use std::sync::Arc;

    const DIMENSION: usize = 16;
    const BASE: u64 = 400;
    // At least 100: the batch path pushes every vector, then links them. The
    // saves are fired once the push is visible, so they run while it links.
    const BATCH: u64 = 200;
    const ROUNDS: u64 = 12;

    let vector = |id: u64| -> Vec<f32> {
        (0..DIMENSION)
            .map(|i| ((id as f32) * 0.37 + (i as f32) * 0.11).sin())
            .collect()
    };

    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_path_buf();
    let index = Arc::new(NativeHnswIndex::new(DIMENSION, DistanceMetric::Euclidean).expect("test"));
    for id in 0..BASE {
        index.insert(id, &vector(id)).expect("test");
    }

    let mut torn = Vec::new();
    let mut unseen = 0;
    let mut previous_generation = 0_u64;
    for round in 0..ROUNDS {
        let first = BASE + round * BATCH;
        let items: Vec<(u64, Vec<f32>)> =
            (first..first + BATCH).map(|id| (id, vector(id))).collect();
        let (seen, saved) = two_saves_while_a_batch_links(&index, &dir_path, items);
        unseen += usize::from(!seen);
        if let Some(err) = saved.into_iter().find_map(Result::err) {
            torn.push(format!("round {round}: save failed: {err}"));
            continue;
        }

        // The mechanism, not only its consequence. Each save stamps the
        // generation after the one it reads from the directory, so two that
        // run one at a time advance it by two. Two that overlap read the
        // same one and both stamp the same value: the directory advances by
        // one, and its files were written twice at the same offsets under
        // that one generation, which is what makes a crash between renames
        // undetectable.
        let generation = crate::index::hnsw::persistence::load_graph_generation(dir.path())
            .expect("test: read the graph generation");
        if generation != previous_generation + 2 {
            torn.push(format!(
                "round {round}: two saves advanced the generation from \
                 {previous_generation} to {generation}, not by two: they read it at the \
                 same time and stamped one value over each other's files"
            ));
        }
        previous_generation = generation;

        match NativeHnswIndex::load(dir.path(), DIMENSION, DistanceMetric::Euclidean) {
            Err(err) => torn.push(format!("round {round}: reload failed: {err}")),
            Ok(loaded) => {
                // The ids inserted before any racing write are in every save
                // either way, so each must still be its own nearest
                // neighbour after the reload. A slot holding another id's
                // vector -- what two saves writing one graph file at the
                // same offsets produce -- breaks exactly that.
                let wrong: Vec<u64> = (0..BASE)
                    .step_by(8)
                    .filter(|&id| {
                        loaded
                            .brute_force_search_parallel(&vector(id), 1)
                            .first()
                            .is_none_or(|hit| hit.id != id)
                    })
                    .collect();
                if !wrong.is_empty() {
                    torn.push(format!(
                        "round {round}: {} of the {} ids sampled are no longer their own \
                         nearest neighbour (first {:?})",
                        wrong.len(),
                        BASE / 8,
                        wrong.first()
                    ));
                }
            }
        }
    }

    // The positive control: rounds whose saves did not run during a linking
    // batch prove nothing about two saves racing one.
    assert_eq!(
        unseen, 0,
        "{unseen} of the {ROUNDS} rounds fired their saves after the batch had ended"
    );
    assert!(
        torn.is_empty(),
        "{} of the {ROUNDS} rounds left a directory two saves had torn: {torn:?}",
        torn.len()
    );
}

/// Inserts `items` as one batch on a thread of its own and, once its vectors
/// are in the arena, saves `index` into `dir` from two threads at once while
/// it links them. Returns whether the push was seen and the two outcomes.
fn two_saves_while_a_batch_links(
    index: &std::sync::Arc<NativeHnswIndex>,
    dir: &std::path::Path,
    items: Vec<(u64, Vec<f32>)>,
) -> (bool, [std::io::Result<()>; 2]) {
    let before = index.len();
    std::thread::scope(|scope| {
        let batch = scope.spawn(|| index.insert_batch_parallel(items));
        // Fire the saves only once the batch's vectors are in the arena, so
        // both run while it is still linking them -- the window in which one
        // graph file is rewritten at the same offsets twice.
        let seen = loop {
            // Read before the count: whatever the batch did, it did before
            // it ended.
            let ended = batch.is_finished();
            if index.len() > before {
                break true;
            }
            if ended {
                break false;
            }
            std::thread::yield_now();
        };
        let save = || {
            let (index, dir) = (std::sync::Arc::clone(index), dir.to_path_buf());
            scope.spawn(move || index.save(&dir))
        };
        let (a, b) = (save(), save());
        batch.join().expect("test: the batch thread panicked");
        (
            seen,
            [
                a.join().expect("test: a saving thread panicked"),
                b.join().expect("test: a saving thread panicked"),
            ],
        )
    })
}
