//! SIMD-optimized vector operations for high-performance distance calculations.
//!
//! # Performance (WIS-45 validated)
//!
//! - `cosine_similarity_fast`: ~83ns for 768d (using explicit SIMD)
//! - `euclidean_distance_fast`: ~47ns for 768d (using explicit SIMD)
//! - `dot_product_fast`: ~45ns for 768d (using explicit SIMD)
//!
//! # Implementation Strategy
//!
//! This module delegates to `simd_explicit` for all distance functions,
//! using the `wide` crate for portable SIMD (AVX2/NEON/WASM).
//!
//! # Note on `hnsw_rs` Integration
//!
//! Custom `Distance` trait implementations for `hnsw_rs` are NOT supported due to
//! undocumented internal invariants in the library. The SIMD functions in this module
//! are used by `DistanceMetric::calculate()` for direct distance computations outside
//! of the HNSW index.

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
/// - **`x86_64`**: Uses `_mm_prefetch` with `_MM_HINT_T0` (all cache levels)
/// - **`aarch64`**: Uses `__prefetch` (ARM NEON)
/// - **Other**: No-op (graceful degradation)
///
/// # Safety
///
/// This function is safe because prefetch instructions are hints and cannot
/// cause memory faults even with invalid addresses.
///
/// # Performance
///
/// Reduces cache miss latency by ~50-100 cycles when vectors are prefetched
/// 4-16 iterations ahead of actual use.
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
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: prefetch is a hint instruction that cannot fault
        unsafe {
            use std::arch::aarch64::_prefetch;
            use std::arch::aarch64::{_PREFETCH_LOCALITY3, _PREFETCH_READ};
            _prefetch(
                vector.as_ptr().cast::<i8>(),
                _PREFETCH_READ,
                _PREFETCH_LOCALITY3,
            );
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        // No-op for unsupported architectures
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
#[inline]
#[must_use]
pub fn hamming_distance_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let chunks = a.len() / 4;
    let remainder = a.len() % 4;

    let mut count0 = 0u32;
    let mut count1 = 0u32;
    let mut count2 = 0u32;
    let mut count3 = 0u32;

    for i in 0..chunks {
        let base = i * 4;
        // Convert to binary: > 0.5 = 1, else 0
        let a0 = a[base] > 0.5;
        let a1 = a[base + 1] > 0.5;
        let a2 = a[base + 2] > 0.5;
        let a3 = a[base + 3] > 0.5;

        let b0 = b[base] > 0.5;
        let b1 = b[base + 1] > 0.5;
        let b2 = b[base + 2] > 0.5;
        let b3 = b[base + 3] > 0.5;

        // XOR to find differences
        count0 += u32::from(a0 != b0);
        count1 += u32::from(a1 != b1);
        count2 += u32::from(a2 != b2);
        count3 += u32::from(a3 != b3);
    }

    // Handle remainder
    let base = chunks * 4;
    for i in 0..remainder {
        let ai = a[base + i] > 0.5;
        let bi = b[base + i] > 0.5;
        count0 += u32::from(ai != bi);
    }

    #[allow(clippy::cast_precision_loss)]
    // Intentional: hamming distance won't exceed 2^23 in practice
    {
        (count0 + count1 + count2 + count3) as f32
    }
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
/// # Optimization
///
/// Uses loop unrolling with 8-element blocks for better instruction-level
/// parallelism. The compiler auto-vectorizes the comparison operations.
#[inline]
#[must_use]
pub fn jaccard_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let mut intersection = 0u32;
    let mut union = 0u32;

    // Process 8 elements at a time (unrolled for ILP)
    let chunks = a.len() / 8;
    for i in 0..chunks {
        let offset = i * 8;

        // Unrolled comparisons for better ILP
        let (int0, uni0) = count_pair(a[offset], b[offset]);
        let (int1, uni1) = count_pair(a[offset + 1], b[offset + 1]);
        let (int2, uni2) = count_pair(a[offset + 2], b[offset + 2]);
        let (int3, uni3) = count_pair(a[offset + 3], b[offset + 3]);
        let (int4, uni4) = count_pair(a[offset + 4], b[offset + 4]);
        let (int5, uni5) = count_pair(a[offset + 5], b[offset + 5]);
        let (int6, uni6) = count_pair(a[offset + 6], b[offset + 6]);
        let (int7, uni7) = count_pair(a[offset + 7], b[offset + 7]);

        intersection += int0 + int1 + int2 + int3 + int4 + int5 + int6 + int7;
        union += uni0 + uni1 + uni2 + uni3 + uni4 + uni5 + uni6 + uni7;
    }

    // Handle remainder
    let remainder_start = chunks * 8;
    for i in remainder_start..a.len() {
        let (int_i, uni_i) = count_pair(a[i], b[i]);
        intersection += int_i;
        union += uni_i;
    }

    // Empty sets are defined as identical (similarity = 1.0)
    if union == 0 {
        return 1.0;
    }

    #[allow(clippy::cast_precision_loss)] // Intentional: set size won't exceed 2^23 in practice
    {
        intersection as f32 / union as f32
    }
}

/// Counts intersection and union contribution for a single element pair.
#[inline]
fn count_pair(a: f32, b: f32) -> (u32, u32) {
    let in_a = a > 0.5;
    let in_b = b > 0.5;
    let intersection = u32::from(in_a && in_b);
    let union = u32::from(in_a || in_b);
    (intersection, union)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    // =========================================================================
    // TDD Tests - Written BEFORE optimization (RED phase)
    // These define the expected behavior and performance contracts.
    // =========================================================================

    const EPSILON: f32 = 1e-5;

    fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
        #[allow(clippy::cast_precision_loss)]
        (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
    }

    // --- Correctness Tests ---

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let result = cosine_similarity_fast(&v, &v);
        assert!(
            (result - 1.0).abs() < EPSILON,
            "Identical vectors should have similarity 1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let result = cosine_similarity_fast(&a, &b);
        assert!(
            result.abs() < EPSILON,
            "Orthogonal vectors should have similarity 0.0"
        );
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        let result = cosine_similarity_fast(&a, &b);
        assert!(
            (result + 1.0).abs() < EPSILON,
            "Opposite vectors should have similarity -1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let result = cosine_similarity_fast(&a, &b);
        assert!(result.abs() < EPSILON, "Zero vector should return 0.0");
    }

    #[test]
    fn test_euclidean_distance_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let result = euclidean_distance_fast(&v, &v);
        assert!(
            result.abs() < EPSILON,
            "Identical vectors should have distance 0.0"
        );
    }

    #[test]
    fn test_euclidean_distance_known_value() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        let result = euclidean_distance_fast(&a, &b);
        assert!(
            (result - 5.0).abs() < EPSILON,
            "Expected distance 5.0 (3-4-5 triangle)"
        );
    }

    #[test]
    fn test_euclidean_distance_768d() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let result = euclidean_distance_fast(&a, &b);

        // Compare with naive implementation
        let expected: f32 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            .sqrt();

        assert!(
            (result - expected).abs() < EPSILON,
            "Should match naive implementation"
        );
    }

    #[test]
    fn test_dot_product_fast_correctness() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![5.0, 6.0, 7.0, 8.0];
        let result = dot_product_fast(&a, &b);
        let expected = 1.0 * 5.0 + 2.0 * 6.0 + 3.0 * 7.0 + 4.0 * 8.0; // 70.0
        assert!((result - expected).abs() < EPSILON);
    }

    #[test]
    fn test_dot_product_fast_768d() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let result = dot_product_fast(&a, &b);
        let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        // Relax epsilon for high-dimensional accumulated floating point errors
        let rel_error = (result - expected).abs() / expected.abs().max(1.0);
        assert!(rel_error < 1e-4, "Relative error too high: {rel_error}");
    }

    #[test]
    fn test_normalize_inplace_unit_vector() {
        let mut v = vec![3.0, 4.0, 0.0];
        normalize_inplace(&mut v);

        let norm_after: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm_after - 1.0).abs() < EPSILON,
            "Normalized vector should have unit norm"
        );
        assert!((v[0] - 0.6).abs() < EPSILON, "Expected 3/5 = 0.6");
        assert!((v[1] - 0.8).abs() < EPSILON, "Expected 4/5 = 0.8");
    }

    #[test]
    fn test_normalize_inplace_zero_vector() {
        let mut v = vec![0.0, 0.0, 0.0];
        normalize_inplace(&mut v);
        // Should not panic, vector unchanged
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_normalize_inplace_768d() {
        let mut v = generate_test_vector(768, 0.0);
        normalize_inplace(&mut v);

        let norm_after: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm_after - 1.0).abs() < EPSILON,
            "Should be unit vector after normalization"
        );
    }

    // --- Consistency Tests (fast vs baseline) ---

    #[test]
    fn test_cosine_consistency_with_baseline() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        // Baseline (3-pass)
        let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        let baseline = dot / (norm_a * norm_b);

        // Fast (single-pass fused)
        let fast = cosine_similarity_fast(&a, &b);

        assert!(
            (fast - baseline).abs() < EPSILON,
            "Fast implementation should match baseline: {fast} vs {baseline}"
        );
    }

    // --- Edge Cases ---

    #[test]
    fn test_odd_dimension_vectors() {
        // Test non-multiple-of-4 dimensions
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // 5 elements
        let b = vec![5.0, 4.0, 3.0, 2.0, 1.0];

        let dot = dot_product_fast(&a, &b);
        let expected = 1.0 * 5.0 + 2.0 * 4.0 + 3.0 * 3.0 + 4.0 * 2.0 + 5.0 * 1.0; // 35.0
        assert!((dot - expected).abs() < EPSILON);

        let dist = euclidean_distance_fast(&a, &b);
        let expected_dist: f32 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!((dist - expected_dist).abs() < EPSILON);
    }

    #[test]
    fn test_small_vectors() {
        // Single element
        let a = vec![3.0];
        let b = vec![4.0];
        assert!((dot_product_fast(&a, &b) - 12.0).abs() < EPSILON);
        assert!((euclidean_distance_fast(&a, &b) - 1.0).abs() < EPSILON);

        // Two elements
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity_fast(&a, &b)).abs() < EPSILON);
    }

    #[test]
    #[should_panic(expected = "Vector dimensions must match")]
    fn test_dimension_mismatch_panics() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let _ = cosine_similarity_fast(&a, &b);
    }

    // --- norm() tests ---

    #[test]
    fn test_norm_zero_vector() {
        let v = vec![0.0, 0.0, 0.0];
        assert!(norm(&v).abs() < EPSILON);
    }

    #[test]
    fn test_norm_unit_vector() {
        let v = vec![1.0, 0.0, 0.0];
        assert!((norm(&v) - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_norm_known_value() {
        let v = vec![3.0, 4.0];
        assert!((norm(&v) - 5.0).abs() < EPSILON);
    }

    // --- squared_l2_distance tests ---

    #[test]
    fn test_squared_l2_identical() {
        let v = vec![1.0, 2.0, 3.0];
        assert!(squared_l2_distance(&v, &v).abs() < EPSILON);
    }

    #[test]
    fn test_squared_l2_known_value() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((squared_l2_distance(&a, &b) - 25.0).abs() < EPSILON);
    }

    // --- hamming_distance_fast tests ---

    #[test]
    fn test_hamming_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        assert!(hamming_distance_fast(&a, &a).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_all_different() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0];
        assert!((hamming_distance_fast(&a, &b) - 4.0).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_partial() {
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0, 1.0];
        assert!((hamming_distance_fast(&a, &b) - 2.0).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_odd_dimension() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0];
        let b = vec![0.0, 0.0, 1.0, 1.0, 1.0];
        assert!((hamming_distance_fast(&a, &b) - 2.0).abs() < EPSILON);
    }

    // --- jaccard_similarity_fast tests ---

    #[test]
    fn test_jaccard_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        assert!((jaccard_similarity_fast(&a, &a) - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        assert!(jaccard_similarity_fast(&a, &b).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_half_overlap() {
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0];
        // Intersection: 1, Union: 3
        assert!((jaccard_similarity_fast(&a, &b) - (1.0 / 3.0)).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_empty_sets() {
        let a = vec![0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0, 0.0];
        assert!((jaccard_similarity_fast(&a, &b) - 1.0).abs() < EPSILON);
    }

    // -------------------------------------------------------------------------
    // TDD: Jaccard SIMD optimization tests (P2)
    // -------------------------------------------------------------------------

    #[test]
    fn test_jaccard_simd_large_vectors() {
        // Test with 768D vectors (typical embedding size)
        let a: Vec<f32> = (0..768)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..768)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();

        let result = jaccard_similarity_fast(&a, &b);

        // Verify result is in valid range
        assert!((0.0..=1.0).contains(&result), "Jaccard must be in [0,1]");
    }

    #[test]
    fn test_jaccard_simd_aligned_vectors() {
        // Test with 8-aligned dimension (optimal for SIMD)
        let a: Vec<f32> = (0..64).map(|i| if i < 32 { 1.0 } else { 0.0 }).collect();
        let b: Vec<f32> = (0..64).map(|i| if i < 48 { 1.0 } else { 0.0 }).collect();

        let result = jaccard_similarity_fast(&a, &b);

        // Intersection: 32 (first 32 elements), Union: 48
        let expected = 32.0 / 48.0;
        assert!(
            (result - expected).abs() < EPSILON,
            "Expected {expected}, got {result}"
        );
    }

    #[test]
    fn test_jaccard_simd_unaligned_vectors() {
        // Test with non-8-aligned dimension (tests remainder handling)
        let a: Vec<f32> = (0..67).map(|i| if i < 30 { 1.0 } else { 0.0 }).collect();
        let b: Vec<f32> = (0..67).map(|i| if i < 40 { 1.0 } else { 0.0 }).collect();

        let result = jaccard_similarity_fast(&a, &b);

        // Intersection: 30, Union: 40
        let expected = 30.0 / 40.0;
        assert!(
            (result - expected).abs() < EPSILON,
            "Expected {expected}, got {result}"
        );
    }

    #[test]
    fn test_jaccard_consistency_scalar_vs_reference() {
        // Property test: verify consistency across different vector sizes
        for dim in [7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 255, 256, 768] {
            let a: Vec<f32> = (0..dim)
                .map(|i| if (i * 7) % 11 < 6 { 1.0 } else { 0.0 })
                .collect();
            let b: Vec<f32> = (0..dim)
                .map(|i| if (i * 5) % 9 < 5 { 1.0 } else { 0.0 })
                .collect();

            let result = jaccard_similarity_fast(&a, &b);

            // Compute reference manually
            let mut intersection = 0u32;
            let mut union = 0u32;
            for i in 0..dim {
                let in_a = a[i] > 0.5;
                let in_b = b[i] > 0.5;
                if in_a && in_b {
                    intersection += 1;
                }
                if in_a || in_b {
                    union += 1;
                }
            }
            let expected = if union == 0 {
                1.0
            } else {
                intersection as f32 / union as f32
            };

            assert!(
                (result - expected).abs() < EPSILON,
                "Dim {dim}: expected {expected}, got {result}"
            );
        }
    }

    // =========================================================================
    // QW-2: Prefetch Helper Tests
    // =========================================================================

    #[test]
    fn test_calculate_prefetch_distance_small_vectors() {
        // 32D vectors (128 bytes) -> raw = 2, clamped to 4
        assert_eq!(calculate_prefetch_distance(32), 4);
        // 64D vectors (256 bytes) -> raw = 4
        assert_eq!(calculate_prefetch_distance(64), 4);
    }

    #[test]
    fn test_calculate_prefetch_distance_medium_vectors() {
        // 128D vectors (512 bytes) -> raw = 8
        assert_eq!(calculate_prefetch_distance(128), 8);
        // 256D vectors (1024 bytes) -> raw = 16
        assert_eq!(calculate_prefetch_distance(256), 16);
    }

    #[test]
    fn test_calculate_prefetch_distance_large_vectors() {
        // 768D vectors (3072 bytes) -> raw = 48, clamped to 16
        assert_eq!(calculate_prefetch_distance(768), 16);
        // 1536D vectors (6144 bytes) -> raw = 96, clamped to 16
        assert_eq!(calculate_prefetch_distance(1536), 16);
    }

    #[test]
    fn test_prefetch_vector_does_not_panic() {
        // Prefetch should never panic, even with edge cases
        let empty: Vec<f32> = vec![];
        prefetch_vector(&empty); // Empty vector - should be no-op

        let small = vec![1.0, 2.0, 3.0];
        prefetch_vector(&small); // Small vector

        let large = generate_test_vector(768, 0.0);
        prefetch_vector(&large); // Large vector
    }

    #[test]
    fn test_l2_cache_line_constant() {
        // Verify constant is set correctly for x86_64
        assert_eq!(L2_CACHE_LINE_BYTES, 64);
    }
}
