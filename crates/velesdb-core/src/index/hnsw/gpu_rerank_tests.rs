//! Tests for GPU reranking pipeline (Phase 2-3).
#![allow(
    clippy::cast_precision_loss,
    clippy::redundant_closure_for_method_calls
)]

use super::index::HnswIndex;
use super::params::SearchQuality;
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
#[cfg(feature = "gpu")]
use std::collections::HashSet;

// =========================================================================
// Step 2.8 — RED: GPU reranking matches SIMD reranking
// =========================================================================

/// Verifies that `rerank_candidates_gpu()` produces scores consistent with
/// the SIMD path for a Cosine metric index.
///
/// Strategy: build an index, run HNSW to get candidates, then compare
/// `rerank_candidates_simd` vs `rerank_candidates_gpu` on those candidates.
/// The GPU path may return `None` when no GPU is available, so the test
/// gracefully skips in that case.
#[test]
#[cfg(feature = "gpu")]
fn test_gpu_rerank_matches_simd() {
    let dim = 128;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Deterministic pseudo-random vectors using simple LCG
    let mut seed: u64 = 42;
    for id in 0u64..200 {
        let v: Vec<f32> = (0..dim)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                // Reason: seed >> 33 fits in u32; normalizing to [-1, 1].
                #[allow(clippy::cast_precision_loss)]
                let val = (seed >> 33) as f32 / u32::MAX as f32 * 2.0 - 1.0;
                val
            })
            .collect();
        index.insert(id, &v);
    }

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.017).sin()).collect();

    // Get HNSW candidates (approximate scores)
    let candidates = index.search_hnsw_only(&query, 50, 128);
    assert!(!candidates.is_empty(), "HNSW should return candidates");

    // SIMD reranking (always available)
    let simd_results = index.rerank_candidates_simd(&query, &candidates);

    // GPU reranking (may return None if GPU unavailable)
    if let Some(gpu_results) = index.rerank_candidates_gpu(&query, &candidates) {
        assert_eq!(
            simd_results.len(),
            gpu_results.len(),
            "GPU and SIMD should produce same number of results"
        );

        // Build a map of id -> score for both results, since order within
        // each result set is by candidate input order (not sorted).
        let simd_map: std::collections::HashMap<u64, f32> =
            simd_results.iter().map(|sr| (sr.id, sr.score)).collect();

        for gpu_sr in &gpu_results {
            let simd_score = simd_map
                .get(&gpu_sr.id)
                .expect("GPU result ID should exist in SIMD results");
            assert!(
                (simd_score - gpu_sr.score).abs() < 0.02,
                "Scores should be close: SIMD={simd_score}, GPU={} for id={}",
                gpu_sr.score,
                gpu_sr.id,
            );
        }
    }
    // If GPU is not available, the test passes trivially (no GPU to test).
}

/// Verifies the end-to-end reranking pipeline with GPU dispatch.
///
/// Uses `search_with_quality(Balanced)` which triggers two-stage reranking.
/// If the GPU threshold is met AND a GPU is present, the GPU path is used
/// transparently. Either way, results must be correct.
#[test]
#[cfg(feature = "gpu")]
fn test_gpu_rerank_end_to_end_balanced_vs_fast() {
    let dim = 128;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    let mut seed: u64 = 7;
    for id in 0u64..500 {
        let v: Vec<f32> = (0..dim)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                #[allow(clippy::cast_precision_loss)]
                let val = (seed >> 33) as f32 / u32::MAX as f32 * 2.0 - 1.0;
                val
            })
            .collect();
        index.insert(id, &v);
    }

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.031).cos()).collect();

    let fast_results = index
        .search_with_quality(&query, 10, SearchQuality::Fast)
        .unwrap();
    let balanced_results = index
        .search_with_quality(&query, 10, SearchQuality::Balanced)
        .unwrap();

    // Both should return results
    assert!(
        !fast_results.is_empty(),
        "Fast search should return results"
    );
    assert!(
        !balanced_results.is_empty(),
        "Balanced search should return results"
    );

    // Balanced results must be sorted descending (Cosine: higher = better)
    for pair in balanced_results.windows(2) {
        assert!(
            pair[0].score >= pair[1].score - f32::EPSILON,
            "Balanced results must be sorted: {} >= {}",
            pair[0].score,
            pair[1].score,
        );
    }

    // Balanced (with reranking) should produce at least as good top-1 score
    // as Fast (no reranking), since exact distances beat approximate ones.
    if !fast_results.is_empty() && !balanced_results.is_empty() {
        assert!(
            balanced_results[0].score >= fast_results[0].score - 0.01,
            "Balanced top-1 ({}) should be >= Fast top-1 ({}) for Cosine",
            balanced_results[0].score,
            fast_results[0].score,
        );
    }
}

// =========================================================================
// Step 2.11 — RED: Fallback below threshold
// =========================================================================

/// Verifies that GPU reranking is NOT used when the workload is below the
/// threshold (rerank_k * dimension <= 262,144).
///
/// Also verifies that search produces correct results via the SIMD path.
#[test]
fn test_gpu_rerank_fallback_below_threshold() {
    #[cfg(feature = "gpu")]
    {
        use crate::gpu::GpuAccelerator;
        // Pure arithmetic — no GPU needed to test the threshold function
        assert!(
            !GpuAccelerator::should_rerank_gpu(5, 4),
            "5 * 4 = 20, should NOT use GPU"
        );
        assert!(
            !GpuAccelerator::should_rerank_gpu(100, 64),
            "100 * 64 = 6400, should NOT use GPU"
        );
    }

    // Verify that search still works correctly with tiny dimensions (SIMD path)
    let index = HnswIndex::new(4, DistanceMetric::Cosine).unwrap();

    index.insert(1, &[1.0, 0.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0, 0.0]);
    index.insert(3, &[0.7, 0.7, 0.0, 0.0]);
    index.insert(4, &[0.0, 0.0, 1.0, 0.0]);
    index.insert(5, &[0.5, 0.5, 0.5, 0.0]);

    let query = [1.0, 0.0, 0.0, 0.0];
    let results = index
        .search_with_quality(&query, 3, SearchQuality::Balanced)
        .unwrap();

    assert!(!results.is_empty(), "Should return results via SIMD path");
    assert_eq!(
        results[0].id, 1,
        "Exact match should be top result, got id={}",
        results[0].id
    );
}

/// Verifies threshold boundary and monotonicity.
///
/// `should_rerank_gpu` returns true when `rerank_k * dimension > 262_144`
/// (strictly greater). Tests the exact boundary and extreme values.
#[test]
#[cfg(feature = "gpu")]
fn test_gpu_rerank_threshold_boundary_and_monotonicity() {
    use crate::gpu::GpuAccelerator;
    // Pure arithmetic — no GPU instance needed

    // Trivial: always false
    assert!(!GpuAccelerator::should_rerank_gpu(1, 1));

    // Exactly at boundary: 2048 * 128 = 262_144, NOT strictly greater
    assert!(
        !GpuAccelerator::should_rerank_gpu(2048, 128),
        "Exactly at threshold (262_144) should NOT trigger GPU"
    );

    // One above: 2049 * 128 = 262_272 > 262_144
    assert!(
        GpuAccelerator::should_rerank_gpu(2049, 128),
        "Above threshold should trigger GPU"
    );

    // Large payload: always true
    assert!(GpuAccelerator::should_rerank_gpu(100_000, 1536));
}

// =========================================================================
// Step 3.1 — RED: Batch search GPU rerank matches CPU
// =========================================================================

/// Deterministic pseudo-random vector generator for reproducible tests.
///
/// Uses a simple LCG to produce vectors in the range [-1, 1].
fn lcg_vector(seed: &mut u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|_| {
            *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            // Reason: seed >> 33 fits in u32; normalizing to [-1, 1].
            #[allow(clippy::cast_precision_loss)]
            let val = (*seed >> 33) as f32 / u32::MAX as f32 * 2.0 - 1.0;
            val
        })
        .collect()
}

/// Verifies that `search_batch_parallel` with GPU reranking produces
/// results consistent with individual `search_with_quality` calls.
///
/// The batch path uses `search_batch_gpu_rerank` under the hood (when GPU
/// is available and conditions are met). Either way, the top-1 result for
/// each query should match between batch and individual paths.
#[test]
#[cfg(feature = "gpu")]
fn test_batch_search_gpu_rerank_matches_cpu() {
    let dim = 128;
    let k = 10;
    let num_vectors = 1000;
    let num_queries = 10;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Insert deterministic vectors
    let mut seed: u64 = 42;
    for id in 0..num_vectors {
        let v = lcg_vector(&mut seed, dim);
        index.insert(id, &v);
    }
    index.set_searching_mode();

    // Create query vectors
    let queries: Vec<Vec<f32>> = (0..num_queries)
        .map(|_| lcg_vector(&mut seed, dim))
        .collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();

    // Batch search (may use GPU reranking if available + threshold met)
    let batch_results = index
        .search_batch_parallel(&query_refs, k, SearchQuality::Balanced)
        .unwrap();

    // Individual searches as baseline
    let individual_results: Vec<Vec<crate::scored_result::ScoredResult>> = queries
        .iter()
        .map(|q| {
            index
                .search_with_quality(q, k, SearchQuality::Balanced)
                .unwrap()
        })
        .collect();

    assert_eq!(batch_results.len(), individual_results.len());

    for (i, (batch, individual)) in batch_results.iter().zip(&individual_results).enumerate() {
        assert_eq!(
            batch.len(),
            individual.len(),
            "Query {i}: result count mismatch"
        );

        // Top-1 should be identical (reranking should not change best match)
        if !batch.is_empty() && !individual.is_empty() {
            assert_eq!(
                batch[0].id, individual[0].id,
                "Query {i}: top-1 ID mismatch (batch={}, individual={})",
                batch[0].id, individual[0].id,
            );
        }

        // Verify all returned IDs overlap significantly (at least 80% of top-k)
        let batch_ids: HashSet<u64> = batch.iter().map(|r| r.id).collect();
        let individual_ids: HashSet<u64> = individual.iter().map(|r| r.id).collect();
        let overlap = batch_ids.intersection(&individual_ids).count();
        // Reason: k is 10, cast to f64 for ratio computation.
        #[allow(clippy::cast_precision_loss)]
        let overlap_ratio = overlap as f64 / k.max(1) as f64;
        assert!(
            overlap_ratio >= 0.8,
            "Query {i}: overlap too low ({overlap}/{k} = {overlap_ratio:.2})"
        );
    }
}

// =========================================================================
// Step 3.3 — RED: Batch search fallback without GPU
// =========================================================================

/// Verifies that `search_batch_parallel` works correctly without GPU
/// (quality=Fast, no reranking).
///
/// This ensures the rayon-only HNSW path remains functional regardless
/// of GPU availability.
#[test]
fn test_batch_search_fallback_without_gpu() {
    let dim = 64;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    let mut seed: u64 = 99;
    for id in 0u64..200 {
        let v = lcg_vector(&mut seed, dim);
        index.insert(id, &v);
    }
    index.set_searching_mode();

    let queries: Vec<Vec<f32>> = (0..5).map(|_| lcg_vector(&mut seed, dim)).collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();

    // Fast quality = no reranking, pure HNSW
    let results = index
        .search_batch_parallel(&query_refs, 5, SearchQuality::Fast)
        .unwrap();

    assert_eq!(results.len(), 5, "Should return one result set per query");
    for (i, result) in results.iter().enumerate() {
        assert_eq!(
            result.len(),
            5,
            "Query {i}: should return exactly 5 neighbors"
        );
        // Verify results are non-empty and have valid IDs
        for sr in result {
            assert!(sr.id < 200, "Query {i}: ID out of range: {}", sr.id);
        }
    }
}

// =========================================================================
// Step 3.4 — Batch Adaptive matches single-query Adaptive
// =========================================================================

/// Verifies that `search_batch_parallel` with `Adaptive` quality produces
/// results consistent with individual `search_with_quality(Adaptive)` calls.
///
/// Adaptive uses spread-based two-phase escalation, which is per-query and
/// not batch-compatible. The batch path delegates to `search_with_quality`
/// per-query to maintain behavioral consistency.
#[test]
fn test_batch_search_adaptive_matches_individual() {
    let dim = 64;
    let k = 5;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    let mut seed: u64 = 77;
    for id in 0u64..500 {
        let v = lcg_vector(&mut seed, dim);
        index.insert(id, &v);
    }
    index.set_searching_mode();

    let adaptive = SearchQuality::Adaptive {
        min_ef: 32,
        max_ef: 256,
    };

    let queries: Vec<Vec<f32>> = (0..8).map(|_| lcg_vector(&mut seed, dim)).collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();

    let batch_results = index
        .search_batch_parallel(&query_refs, k, adaptive)
        .unwrap();
    let individual_results: Vec<Vec<crate::scored_result::ScoredResult>> = queries
        .iter()
        .map(|q| index.search_with_quality(q, k, adaptive).unwrap())
        .collect();

    assert_eq!(batch_results.len(), individual_results.len());

    for (i, (batch, individual)) in batch_results.iter().zip(&individual_results).enumerate() {
        assert_eq!(
            batch.len(),
            individual.len(),
            "Query {i}: result count mismatch"
        );

        // Top-1 must match (same algorithm, same data)
        if !batch.is_empty() && !individual.is_empty() {
            assert_eq!(
                batch[0].id, individual[0].id,
                "Query {i}: top-1 ID mismatch (batch={}, individual={})",
                batch[0].id, individual[0].id,
            );
        }
    }
}

// =========================================================================
// Step 4.1 — RED: GPU brute-force parallel matches rayon
// =========================================================================

/// Verifies that `brute_force_search_gpu_dispatch` produces the same top-k
/// results as the rayon-based `brute_force_search_parallel`.
///
/// Uses `brute_force_search_gpu_dispatch` directly (bypassing the 100K
/// threshold in `brute_force_search_parallel`) so the test can run with
/// a smaller dataset.
#[test]
#[cfg(feature = "gpu")]
fn test_brute_force_gpu_matches_rayon() {
    let dim = 128;
    let num_vectors: u64 = 5_000;
    let k = 10;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Insert deterministic vectors using shared LCG helper
    let mut seed: u64 = 42;
    for id in 0..num_vectors {
        let v = lcg_vector(&mut seed, dim);
        index.insert(id, &v);
    }
    index.set_searching_mode();

    let query = lcg_vector(&mut seed, dim);

    // Rayon path (always available)
    let rayon_results = index.brute_force_search_parallel(&query, k).unwrap();

    // GPU path (may return None if GPU unavailable)
    if let Some(gpu_results) = index.brute_force_search_gpu_dispatch(&query, k) {
        assert_eq!(
            gpu_results.len(),
            rayon_results.len(),
            "GPU and rayon should produce same result count"
        );

        // Top-3 IDs should be identical (exact search, same vectors)
        for rank in 0..3.min(rayon_results.len()) {
            assert_eq!(
                gpu_results[rank].id, rayon_results[rank].id,
                "Rank {rank}: GPU id={} vs rayon id={}",
                gpu_results[rank].id, rayon_results[rank].id,
            );
        }

        // All scores should be close (GPU vs SIMD floating-point differences)
        let rayon_map: std::collections::HashMap<u64, f32> =
            rayon_results.iter().map(|sr| (sr.id, sr.score)).collect();

        for gpu_sr in &gpu_results {
            if let Some(&rayon_score) = rayon_map.get(&gpu_sr.id) {
                assert!(
                    (rayon_score - gpu_sr.score).abs() < 0.02,
                    "Scores diverge: rayon={rayon_score}, GPU={} for id={}",
                    gpu_sr.score,
                    gpu_sr.id,
                );
            }
        }
    }
    // If GPU is unavailable, test passes trivially.
}
