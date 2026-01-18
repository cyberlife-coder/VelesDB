//! SIMD-optimized vector operations for high-performance distance calculations.
//!
//! # Performance (January 2026 benchmarks)
//!
//! - `dot_product_fast`: **66ns** for 1536d (8x speedup vs scalar)
//! - `cosine_similarity_fast`: ~70ns for 768d
//! - `euclidean_distance_fast`: ~45ns for 768d
//!
//! # Implementation Strategy
//!
//! This module delegates to `simd_explicit` and `simd_native` for all distance functions,
//! using AVX-512/AVX2 native intrinsics with `wide` crate fallback for portability.

use crate::simd_avx512;
use crate::simd_explicit;

// ============================================================================
// CPU Cache Prefetch Utilities (QW-2 Refactoring)
// ============================================================================

/// L2 cache line size in bytes (standard for modern `x86_64` CPUs).
pub const L2_CACHE_LINE_BYTES: usize = 64;

/// Calculates optimal prefetch distance based on vector dimension.
///
/// # Algorithm
///
/// Prefetch distance is computed to stay within L2 cache constraints:
/// - `distance = (vector_bytes / L2_CACHE_LINE).clamp(4, 16)`
/// - Minimum 4: Ensure enough lookahead for out-of-order execution
/// - Maximum 16: Prevent cache pollution from over-prefetching
///
/// # Performance Impact
///
/// - `768D` vectors (3072 bytes): `prefetch_distance` = 16
/// - `128D` vectors (512 bytes): `prefetch_distance` = 8
/// - `32D` vectors (128 bytes): `prefetch_distance` = 4
#[inline]
#[must_use]
pub const fn calculate_prefetch_distance(dimension: usize) -> usize {
    let vector_bytes = dimension * std::mem::size_of::<f32>();
    let raw_distance = vector_bytes / L2_CACHE_LINE_BYTES;
    // Manual clamp for const fn (clamp is not const in stable Rust)
    if raw_distance < 4 {
        4
    } else if raw_distance > 16 {
        16
    } else {
        raw_distance
    }
}

/// Prefetches a vector into L1 cache (T0 hint) for upcoming SIMD operations.
///
/// # Platform Support
///
/// - **`x86_64`**: Uses `_mm_prefetch` with `_MM_HINT_T0` (all cache levels) ✅
/// - **`aarch64`**: No-op (see limitation below) ⚠️
/// - **Other**: No-op (graceful degradation)
///
/// # ARM64 Limitation (rust-lang/rust#117217)
///
/// ARM NEON prefetch intrinsics (`__prefetch`) require the unstable feature
/// `stdarch_aarch64_prefetch` which is not available on stable Rust.
/// When this feature stabilizes, we can enable prefetch for ARM64 platforms
/// (Apple Silicon, ARM servers) for an estimated +10-20% performance gain.
///
/// Tracking: <https://github.com/rust-lang/rust/issues/117217>
///
/// # Safety
///
/// This function is safe because prefetch instructions are hints and cannot
/// cause memory faults even with invalid addresses.
///
/// # Performance
///
/// On `x86_64`: Reduces cache miss latency by ~50-100 cycles when vectors are
/// prefetched 4-16 iterations ahead of actual use.
#[inline]
pub fn prefetch_vector(vector: &[f32]) {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: _mm_prefetch is a hint instruction that cannot fault
        unsafe {
            use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
            _mm_prefetch(vector.as_ptr().cast::<i8>(), _MM_HINT_T0);
        }
    }

    // ARM64 prefetch: blocked on rust-lang/rust#117217 (stdarch_aarch64_prefetch)
    // When stabilized, uncomment the following:
    // #[cfg(target_arch = "aarch64")]
    // {
    //     unsafe {
    //         use std::arch::aarch64::_prefetch;
    //         _prefetch(vector.as_ptr().cast::<i8>(), _PREFETCH_READ, _PREFETCH_LOCALITY3);
    //     }
    // }

    #[cfg(not(target_arch = "x86_64"))]
    {
        // No-op for ARM64 (until stabilization) and other architectures
        let _ = vector;
    }
}

/// Computes cosine similarity using explicit SIMD (f32x8).
///
/// # Algorithm
///
/// Single-pass fused computation of dot(a,b), norm(a)², norm(b)² using SIMD FMA.
/// Result: `dot / (sqrt(norm_a) * sqrt(norm_b))`
///
/// # Performance
///
/// ~83ns for 768d vectors (3.9x faster than auto-vectorized version).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn cosine_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    // Use 32-wide optimized version for large vectors (768D+)
    simd_avx512::cosine_similarity_auto(a, b)
}

/// Computes euclidean distance using explicit SIMD (f32x8).
///
/// # Performance
///
/// ~47ns for 768d vectors (2.9x faster than auto-vectorized version).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn euclidean_distance_fast(a: &[f32], b: &[f32]) -> f32 {
    // Use 32-wide optimized version for large vectors
    simd_avx512::euclidean_auto(a, b)
}

/// Computes squared L2 distance (avoids sqrt for comparison purposes).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn squared_l2_distance(a: &[f32], b: &[f32]) -> f32 {
    // Use 32-wide optimized version for large vectors
    simd_avx512::squared_l2_auto(a, b)
}

/// Normalizes a vector in-place using explicit SIMD.
///
/// # Panics
///
/// Does not panic on zero vector (leaves unchanged).
#[inline]
pub fn normalize_inplace(v: &mut [f32]) {
    simd_explicit::normalize_inplace_simd(v);
}

/// Computes the L2 norm (magnitude) of a vector.
#[inline]
#[must_use]
pub fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Computes dot product using explicit SIMD (f32x8).
///
/// # Performance
///
/// ~45ns for 768d vectors (2.9x faster than auto-vectorized version).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn dot_product_fast(a: &[f32], b: &[f32]) -> f32 {
    // Use 32-wide optimized version for large vectors (768D+)
    simd_avx512::dot_product_auto(a, b)
}

/// Cosine similarity for pre-normalized unit vectors (fast path).
///
/// **IMPORTANT**: Both vectors MUST be pre-normalized (||a|| = ||b|| = 1).
/// If vectors are not normalized, use `cosine_similarity_fast` instead.
///
/// # Performance
///
/// ~40% faster than `cosine_similarity_fast` for 768D vectors because:
/// - Skips norm computation (saves 2 SIMD reductions)
/// - Only computes dot product
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn cosine_similarity_normalized(a: &[f32], b: &[f32]) -> f32 {
    simd_avx512::cosine_similarity_normalized(a, b)
}

/// Batch cosine similarities for pre-normalized vectors with prefetching.
///
/// # Performance
///
/// - Uses CPU prefetch hints for cache warming
/// - ~40% faster per vector than non-normalized version
#[must_use]
pub fn batch_cosine_normalized(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    simd_avx512::batch_cosine_normalized(candidates, query)
}

/// Computes Hamming distance for binary vectors.
///
/// Counts the number of positions where values differ (treating values > 0.5 as 1, else 0).
///
/// # Arguments
///
/// * `a` - First binary vector (values > 0.5 treated as 1)
/// * `b` - Second binary vector (values > 0.5 treated as 1)
///
/// # Returns
///
/// Number of positions where bits differ.
///
/// # Panics
///
/// Panics if vectors have different lengths.
///
/// # Performance (PERF-1 fix v0.8.2)
///
/// Delegates to `simd_explicit::hamming_distance_simd` for guaranteed SIMD
/// vectorization. Previous scalar implementation suffered from auto-vectorization
/// being broken by compiler heuristics.
#[inline]
#[must_use]
pub fn hamming_distance_fast(a: &[f32], b: &[f32]) -> f32 {
    // PERF-1: Delegate to explicit SIMD to avoid auto-vectorization issues
    crate::simd_explicit::hamming_distance_simd(a, b)
}

/// Computes Jaccard similarity for set-like vectors.
///
/// Measures intersection over union of non-zero elements.
/// Values > 0.5 are considered "in the set".
///
/// # Arguments
///
/// * `a` - First set vector (values > 0.5 treated as set members)
/// * `b` - Second set vector (values > 0.5 treated as set members)
///
/// # Returns
///
/// Jaccard similarity in range [0.0, 1.0]. Returns 1.0 for two empty sets.
///
/// # Panics
///
/// Panics if vectors have different lengths.
///
/// # Performance (PERF-1 fix v0.8.2)
///
/// Delegates to `simd_explicit::jaccard_similarity_simd` for guaranteed SIMD
/// vectorization. Previous scalar implementation suffered from auto-vectorization
/// being broken by compiler heuristics (+650% regression).
#[inline]
#[must_use]
pub fn jaccard_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    // PERF-1: Delegate to explicit SIMD to avoid auto-vectorization issues
    crate::simd_explicit::jaccard_similarity_simd(a, b)
}
