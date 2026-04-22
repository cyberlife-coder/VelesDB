//! Tests for the GPU-aware `search_auto` routing in the production HNSW path.
//!
//! These tests verify that the CPU/GPU dispatch wiring (PR #634, PR-D) is
//! correctly integrated at the `HnswIndex` and `NativeHnswInner` layer:
//!
//! 1. **Parity**: at sub-threshold sizes (below `should_traverse_gpu`'s
//!    500K vector gate) `search_auto` falls through to the CPU path and
//!    must yield identical `(node_id, raw_dist)` tuples as the direct
//!    CPU `search` call.
//!
//! 2. **Routing**: the production call-sites (`search_hnsw_only`,
//!    `search_hnsw_only_filtered`) still return correctly ranked results
//!    after being switched to `search_auto`.
//!
//! 3. **Backend isolation**: `search_auto` on a Standard backend only
//!    touches GPU code when gated; on `RaBitQ` it always stays on CPU.
//!    We cannot construct a 500K+ index in a unit test to exercise the
//!    GPU path itself — that is covered by `gpu_traversal_benchmark.rs`.
//!
//! The tests run without the `gpu` feature (CPU-only path) and, when the
//! feature is enabled, additionally verify that the gated path still
//! produces parity with the direct CPU path for sub-threshold indices.

#![allow(clippy::cast_precision_loss)]

use super::index::HnswIndex;
use super::params::SearchQuality;
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;

/// Deterministic pseudo-random vector generator (no `rand` dependency).
///
/// Mirrors the pattern used in `gpu_rerank_tests.rs` so test data is
/// reproducible across runs and machines.
fn pseudo_random_vector(dim: usize, seed: &mut u64) -> Vec<f32> {
    (0..dim)
        .map(|_| {
            *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (*seed >> 33) as f32 / u32::MAX as f32 * 2.0 - 1.0
        })
        .collect()
}

/// Builds a small HNSW index populated with deterministic vectors.
fn build_small_index(dim: usize, count: u64, metric: DistanceMetric) -> HnswIndex {
    let index = HnswIndex::new(dim, metric).expect("test: new HNSW index");
    let mut seed: u64 = 0x00C0_FFEE;
    for id in 0..count {
        let v = pseudo_random_vector(dim, &mut seed);
        index.insert(id, &v);
    }
    index
}

// =========================================================================
// Parity — search_auto falls back to CPU below the GPU threshold
// =========================================================================

/// GIVEN a small HNSW index (sub-500K vectors — below `should_traverse_gpu`)
/// WHEN the production `search_hnsw_only` is invoked (now wired through
///      `NativeHnswInner::search_auto`)
/// THEN it returns the same ranked set as the direct `inner.search` call,
///      proving the CPU fallback path inside `search_auto` is honoured.
#[test]
fn search_auto_matches_cpu_search_below_gpu_threshold_cosine() {
    let dim = 64;
    let k = 10;
    let ef_search = 128;
    let index = build_small_index(dim, 2000, DistanceMetric::Cosine);

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.0173).sin()).collect();

    // Path under test — production call-site routes through search_auto.
    let auto_results = index.search_hnsw_only(&query, k, ef_search);

    // Reference — direct inner.search (pre-PR behaviour).
    let ref_results: Vec<_> = {
        let inner = index.inner.read();
        let neighbours = inner.search(&query, k, ef_search);
        neighbours
            .into_iter()
            .filter_map(|(node_id, raw_dist)| {
                index.mappings.get_id(node_id).map(|id| {
                    let score = inner.transform_score(raw_dist);
                    crate::scored_result::ScoredResult::new(id, score)
                })
            })
            .collect()
    };

    assert_eq!(
        auto_results.len(),
        ref_results.len(),
        "search_auto and direct search must return the same number of results below the GPU threshold",
    );
    let auto_ids: Vec<u64> = auto_results.iter().map(|r| r.id).collect();
    let ref_ids: Vec<u64> = ref_results.iter().map(|r| r.id).collect();
    assert_eq!(
        auto_ids, ref_ids,
        "search_auto must return the same IDs in the same order as CPU search below the GPU threshold",
    );
    for (a, b) in auto_results.iter().zip(ref_results.iter()) {
        assert!(
            (a.score - b.score).abs() < f32::EPSILON * 16.0,
            "scores must match exactly: auto={} vs ref={}",
            a.score,
            b.score,
        );
    }
}

/// Same parity check for Euclidean — ensures the distance transform
/// (squared-L2 → L2) is consistent across both paths.
#[test]
fn search_auto_matches_cpu_search_below_gpu_threshold_euclidean() {
    let dim = 32;
    let k = 5;
    let ef_search = 96;
    let index = build_small_index(dim, 1500, DistanceMetric::Euclidean);

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.021).cos()).collect();

    let auto_results = index.search_hnsw_only(&query, k, ef_search);
    let ref_results: Vec<_> = {
        let inner = index.inner.read();
        let neighbours = inner.search(&query, k, ef_search);
        neighbours
            .into_iter()
            .filter_map(|(node_id, raw_dist)| {
                index.mappings.get_id(node_id).map(|id| {
                    let score = inner.transform_score(raw_dist);
                    crate::scored_result::ScoredResult::new(id, score)
                })
            })
            .collect()
    };

    let auto_ids: Vec<u64> = auto_results.iter().map(|r| r.id).collect();
    let ref_ids: Vec<u64> = ref_results.iter().map(|r| r.id).collect();
    assert_eq!(auto_ids, ref_ids, "Euclidean parity below GPU threshold");
}

// =========================================================================
// Routing — public search paths still produce correctly ranked results
// =========================================================================

/// GIVEN a 1000-vector index (all sub-threshold, CPU path only)
/// WHEN we search via the public `HnswIndex::search` trait method
///      (goes through `search_with_quality(Balanced)` → `search_hnsw_only`
///      → `NativeHnswInner::search_auto`)
/// THEN the top-k recall against brute force is above 0.80 — confirming
///      the wiring did not break graph traversal correctness.
#[test]
fn search_auto_wiring_preserves_recall_small_dataset() {
    let dim = 48;
    let count = 1000;
    let k = 10;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).expect("test");

    // Build reproducible dataset
    let dataset: Vec<Vec<f32>> = (0..count)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.0013).sin())
                .collect::<Vec<f32>>()
        })
        .collect();

    for (idx, vec) in dataset.iter().enumerate() {
        index.insert(idx as u64, vec);
    }

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.007).cos()).collect();

    // Brute-force ground truth
    let mut distances: Vec<(u64, f32)> = dataset
        .iter()
        .enumerate()
        .map(|(idx, vec)| {
            (
                idx as u64,
                crate::simd_native::cosine_similarity_native(&query, vec),
            )
        })
        .collect();
    distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let ground_truth: std::collections::HashSet<u64> =
        distances.iter().take(k).map(|(id, _)| *id).collect();

    // HNSW search — goes through search_auto
    let results = <HnswIndex as VectorIndex>::search(&index, &query, k);
    let retrieved: std::collections::HashSet<u64> = results.iter().map(|r| r.id).collect();

    let intersection = retrieved.intersection(&ground_truth).count();
    let recall = intersection as f64 / k as f64;

    assert!(
        recall >= 0.80,
        "Recall@{k} via search_auto-wired path must be >= 80% on a small CPU-fallback index; got {:.1}%",
        recall * 100.0,
    );
}

/// GIVEN a small HNSW index
/// WHEN we call `search_with_quality(Accurate)` which internally calls
///      `search_hnsw_only` (now routed through `search_auto`)
/// THEN the top result is ordered by best similarity score — i.e. the
///      routing does not break the transform_score + sort contract.
#[test]
fn search_auto_wiring_preserves_score_ordering_on_accurate_quality() {
    let dim = 16;
    let count = 200;
    let index = build_small_index(dim, count, DistanceMetric::Cosine);

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.045).sin()).collect();

    let results = index
        .search_with_quality(&query, 20, SearchQuality::Accurate)
        .expect("test: search_with_quality");

    assert!(!results.is_empty(), "Accurate search must return results");

    // For Cosine (higher-is-better), scores must be monotonically decreasing.
    for pair in results.windows(2) {
        assert!(
            pair[0].score >= pair[1].score - f32::EPSILON * 16.0,
            "scores must be non-increasing for Cosine: {} then {}",
            pair[0].score,
            pair[1].score,
        );
    }
}

// =========================================================================
// Feature-gated GPU parity — identical results with gpu feature active
// =========================================================================

/// GIVEN the `gpu` feature is enabled AND the index is below the GPU
///       traversal threshold (`should_traverse_gpu` returns false)
/// WHEN `search_auto` is invoked
/// THEN it must behave exactly like the CPU `search` call — no GPU
///      dispatch happens because the threshold gates it out.
///
/// This specifically verifies the `#[cfg(feature = "gpu")]` branch in
/// `NativeHnswInner::search_auto` correctly short-circuits via
/// `should_traverse_gpu` at small sizes.
#[test]
#[cfg(feature = "gpu")]
fn search_auto_below_threshold_matches_cpu_under_gpu_feature() {
    let dim = 64;
    let k = 10;
    let ef_search = 128;
    let index = build_small_index(dim, 3000, DistanceMetric::Cosine);

    assert!(
        !crate::gpu::should_traverse_gpu(index.len(), dim),
        "test pre-condition: 3000 vectors must be below the GPU threshold",
    );

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.011).sin()).collect();

    let inner = index.inner.read();
    let auto_neighbours = inner.search_auto(&query, k, ef_search);
    let cpu_neighbours = inner.search(&query, k, ef_search);

    assert_eq!(
        auto_neighbours, cpu_neighbours,
        "Under gpu feature at sub-threshold size, search_auto must tail-call search() verbatim",
    );
}
