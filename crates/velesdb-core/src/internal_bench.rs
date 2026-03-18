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
        Some(unsafe { crate::simd_native::cosine_fused_avx512(a, b) })
    } else {
        None
    }
}

/// Sparse batch insert helper used by internal benches.
pub fn sparse_insert_batch(index: &SparseInvertedIndex, docs: &[(u64, SparseVector)]) {
    index.insert_batch_chunk(docs);
}

/// Parses a query through the cache without recording stats.
pub fn velesql_parse_without_stats(cache: &QueryCache, query: &str) -> Result<Query, ParseError> {
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
