//! Hidden bench-only helpers for internal performance comparisons.

use crate::simd_native::{cosine_similarity_native, DistanceEngine, SimdLevel};
use crate::sparse_index::{SparseInvertedIndex, SparseVector};
use crate::velesql::{ParseError, Query, QueryCache};
use std::hash::{BuildHasher, Hasher};

/// Scalar cosine baseline used by internal benches.
#[must_use]
pub fn cosine_scalar(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::scalar::cosine_scalar(a, b)
}

/// Public dispatch cosine path used by internal benches.
#[must_use]
pub fn cosine_dispatch(a: &[f32], b: &[f32]) -> f32 {
    cosine_similarity_native(a, b)
}

/// Pre-resolved cosine path used by internal benches.
#[must_use]
pub fn cosine_resolved(a: &[f32], b: &[f32]) -> f32 {
    let engine = DistanceEngine::new(a.len());
    engine.cosine_similarity(a, b)
}

/// Returns the runtime SIMD level cached for this process.
#[must_use]
pub fn detected_simd_level() -> SimdLevel {
    crate::simd_native::simd_level()
}

/// Direct AVX2 2-acc cosine kernel when supported by the current CPU.
#[cfg(target_arch = "x86_64")]
#[must_use]
pub fn cosine_avx2_2acc(a: &[f32], b: &[f32]) -> Option<f32> {
    if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma") {
        // SAFETY: Feature detection above guarantees AVX2+FMA availability.
        // - Condition 1: `is_x86_feature_detected!("avx2")` confirms AVX2 support.
        // - Condition 2: `is_x86_feature_detected!("fma")` confirms FMA support.
        // SAFETY: Direct call to AVX2 2-accumulator cosine kernel for benchmarking.
        Some(unsafe { crate::simd_native::cosine_fused_avx2_2acc(a, b) })
    } else {
        None
    }
}

/// Direct AVX2 4-acc cosine kernel when supported by the current CPU.
#[cfg(target_arch = "x86_64")]
#[must_use]
pub fn cosine_avx2_4acc(a: &[f32], b: &[f32]) -> Option<f32> {
    if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma") {
        // SAFETY: Feature detection above guarantees AVX2+FMA availability.
        // - Condition 1: `is_x86_feature_detected!("avx2")` confirms AVX2 support.
        // - Condition 2: `is_x86_feature_detected!("fma")` confirms FMA support.
        // SAFETY: Direct call to AVX2 4-accumulator cosine kernel for benchmarking.
        Some(unsafe { crate::simd_native::cosine_fused_avx2(a, b) })
    } else {
        None
    }
}

/// Direct AVX-512 cosine kernel when supported by the current CPU.
#[cfg(target_arch = "x86_64")]
#[must_use]
pub fn cosine_avx512(a: &[f32], b: &[f32]) -> Option<f32> {
    if std::arch::is_x86_feature_detected!("avx512f") {
        // SAFETY: Feature detection above guarantees AVX-512F availability.
        // - Condition 1: `is_x86_feature_detected!("avx512f")` confirms AVX-512F support.
        // SAFETY: Direct call to AVX-512 cosine kernel for benchmarking.
        Some(unsafe { crate::simd_native::cosine_fused_avx512(a, b) })
    } else {
        None
    }
}

/// Sparse batch insert helper used by internal benches.
pub fn sparse_insert_batch(index: &SparseInvertedIndex, docs: &[(u64, SparseVector)]) {
    index.insert_batch_chunk(docs);
}

/// Runs the linear-scan strategy directly, bypassing `sparse_search`'s router.
///
/// `sparse_search` picks a strategy from `doc_count`, so it cannot be asked
/// for the other one at a given corpus size — which is exactly what #2177's
/// open question needs: above `SMALL_CORPUS_LINEAR_THRESHOLD`, does
/// `maxscore_search` beat the linear scan, or has it only ever been the
/// unmeasured branch? These two entry points let a harness score the same
/// corpus and the same query both ways and compare the work.
///
/// A benchmark-only seam, like `HnswIndex::search_raw` under `bench-sift1m`:
/// not part of the stable API, and the router stays the only way in for
/// application code.
#[must_use]
pub fn sparse_linear_scan_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<crate::index::sparse::ScoredDoc> {
    crate::sparse_index::search::linear_scan_search_for_bench(index, query, k)
}

/// Runs the `MaxScore` DAAT strategy directly, bypassing the router.
///
/// See [`sparse_linear_scan_search`] for why this seam exists.
#[must_use]
pub fn sparse_maxscore_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<crate::index::sparse::ScoredDoc> {
    crate::sparse_index::search::maxscore_search_for_bench(index, query, k)
}

/// Returns the sparse posting inspections counted since the last reset.
///
/// See `sparse_index::op_count` for what one inspection is, and for why a
/// wall-clock figure taken under this feature is not a benchmark.
#[must_use]
pub fn sparse_scoring_ops() -> u64 {
    crate::sparse_index::op_count::scoring_ops()
}

/// Resets the sparse posting-inspection counter to zero.
pub fn reset_sparse_scoring_ops() {
    crate::sparse_index::op_count::reset_scoring_ops();
}

/// Parses a query through the cache without recording stats.
pub fn velesql_parse_without_stats(
    cache: &QueryCache,
    query: &str,
) -> Result<std::sync::Arc<Query>, ParseError> {
    cache.parse_without_stats(query)
}

/// Computes canonicalize + hash cost using the same Fx hasher family as `QueryCache`.
#[must_use]
pub fn velesql_canonical_hash(query: &str) -> u64 {
    let canonical = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut hasher = rustc_hash::FxBuildHasher.build_hasher();
    hasher.write(canonical.as_bytes());
    hasher.finish()
}

// =============================================================================
// Deterministic distance-evaluation counter (cost-crossover harness)
// =============================================================================

/// Returns the process-global count of single-pair distance evaluations.
///
/// See `index::hnsw::eval_count` for what is (and is not) counted. The count
/// is a deterministic work measure for seeded corpora — usable on shared CI
/// runners where wall-clock is noise.
#[cfg(feature = "persistence")]
#[must_use]
pub fn hnsw_distance_evals() -> u64 {
    crate::index::hnsw::eval_count::distance_evals()
}

/// Resets the distance-evaluation counter to zero.
#[cfg(feature = "persistence")]
pub fn reset_hnsw_distance_evals() {
    crate::index::hnsw::eval_count::reset_distance_evals();
}

/// Ids a vacuum's swap copied under the write guard since the last reset.
///
/// See `index::hnsw::index::swap_count` for the unit and why it exists
/// (#2335).
#[must_use]
pub fn vacuum_reconciled_under_guard() -> u64 {
    crate::index::hnsw::index::swap_count::reconciled_under_guard()
}

/// The largest single swap's count since the last reset.
#[must_use]
pub fn vacuum_peak_reconciled_under_guard() -> u64 {
    crate::index::hnsw::index::swap_count::peak_reconciled_under_guard()
}

/// Ids a vacuum settled under its seal, outside the graph guard.
#[must_use]
pub fn vacuum_settled_under_seal() -> u64 {
    crate::index::hnsw::index::swap_count::settled_under_seal()
}

/// Resets every vacuum swap counter.
pub fn reset_vacuum_reconciled_under_guard() {
    crate::index::hnsw::index::swap_count::reset_reconciled_under_guard();
}
