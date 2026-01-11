//! Native SIMD intrinsics for maximum performance.
//!
//! This module provides hand-tuned SIMD implementations using `core::arch` intrinsics
//! for AVX-512, AVX2, and ARM NEON architectures.
//!
//! # Performance (based on arXiv research)
//!
//! - **AVX-512**: True 16-wide f32 operations (vs 4×f32x8 in wide crate)
//! - **ARM NEON**: Native 128-bit SIMD for Apple Silicon/ARM64
//! - **Prefetch**: Software prefetching for cache optimization
//!
//! # References
//!
//! - arXiv:2505.07621 "Bang for the Buck: Vector Search on Cloud CPUs"
//! - arXiv:2502.18113 "Accelerating Graph Indexing for ANNS on Modern CPUs"

// Allow AVX-512 intrinsics even if MSRV is lower (runtime feature detection ensures safety)
#![allow(clippy::incompatible_msrv)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::similar_names)]

// =============================================================================
// AVX-512 Implementation (x86_64)
// =============================================================================

/// AVX-512 dot product using native intrinsics.
///
/// Processes 16 floats per iteration using `_mm512_fmadd_ps`.
/// Falls back to AVX2 or scalar if AVX-512 not available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
unsafe fn dot_product_avx512(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16;

    let mut sum = _mm512_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va = _mm512_loadu_ps(a_ptr.add(offset));
        let vb = _mm512_loadu_ps(b_ptr.add(offset));
        sum = _mm512_fmadd_ps(va, vb, sum);
    }

    let mut result = _mm512_reduce_add_ps(sum);

    // Handle remainder
    let base = simd_len * 16;
    for i in base..len {
        result += a[i] * b[i];
    }

    result
}

/// AVX-512 squared L2 distance.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
unsafe fn squared_l2_avx512(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16;

    let mut sum = _mm512_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va = _mm512_loadu_ps(a_ptr.add(offset));
        let vb = _mm512_loadu_ps(b_ptr.add(offset));
        let diff = _mm512_sub_ps(va, vb);
        sum = _mm512_fmadd_ps(diff, diff, sum);
    }

    let mut result = _mm512_reduce_add_ps(sum);

    // Handle remainder
    let base = simd_len * 16;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}

/// AVX-512 cosine similarity for pre-normalized vectors.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
#[inline]
#[allow(dead_code)] // Reserved for future direct use
unsafe fn cosine_normalized_avx512(a: &[f32], b: &[f32]) -> f32 {
    // For unit vectors: cos(θ) = a · b
    dot_product_avx512(a, b)
}

// =============================================================================
// AVX2 Implementation (x86_64 fallback)
// =============================================================================

/// AVX2 dot product with 2 accumulators for ILP.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16; // Process 16 per iteration (2×8)

    let mut sum0 = _mm256_setzero_ps();
    let mut sum1 = _mm256_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va0 = _mm256_loadu_ps(a_ptr.add(offset));
        let vb0 = _mm256_loadu_ps(b_ptr.add(offset));
        sum0 = _mm256_fmadd_ps(va0, vb0, sum0);

        let va1 = _mm256_loadu_ps(a_ptr.add(offset + 8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(offset + 8));
        sum1 = _mm256_fmadd_ps(va1, vb1, sum1);
    }

    // Combine accumulators
    let combined = _mm256_add_ps(sum0, sum1);

    // Horizontal sum: [a0,a1,a2,a3,a4,a5,a6,a7] -> scalar
    let hi = _mm256_extractf128_ps(combined, 1);
    let lo = _mm256_castps256_ps128(combined);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    // Handle remainder
    let base = simd_len * 16;
    for i in base..len {
        result += a[i] * b[i];
    }

    result
}

/// AVX2 squared L2 distance.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "fma")]
#[inline]
unsafe fn squared_l2_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let len = a.len();
    let simd_len = len / 16;

    let mut sum0 = _mm256_setzero_ps();
    let mut sum1 = _mm256_setzero_ps();

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 16;
        let va0 = _mm256_loadu_ps(a_ptr.add(offset));
        let vb0 = _mm256_loadu_ps(b_ptr.add(offset));
        let diff0 = _mm256_sub_ps(va0, vb0);
        sum0 = _mm256_fmadd_ps(diff0, diff0, sum0);

        let va1 = _mm256_loadu_ps(a_ptr.add(offset + 8));
        let vb1 = _mm256_loadu_ps(b_ptr.add(offset + 8));
        let diff1 = _mm256_sub_ps(va1, vb1);
        sum1 = _mm256_fmadd_ps(diff1, diff1, sum1);
    }

    let combined = _mm256_add_ps(sum0, sum1);
    let hi = _mm256_extractf128_ps(combined, 1);
    let lo = _mm256_castps256_ps128(combined);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let mut result = _mm_cvtss_f32(_mm_add_ss(sums, shuf2));

    let base = simd_len * 16;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}

// =============================================================================
// ARM NEON Implementation (aarch64)
// =============================================================================

/// ARM NEON dot product using native intrinsics.
#[cfg(target_arch = "aarch64")]
#[inline]
fn dot_product_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            sum = vfmaq_f32(sum, va, vb);
        }
    }

    // Horizontal sum
    let mut result = unsafe { vaddvq_f32(sum) };

    // Handle remainder
    let base = simd_len * 4;
    for i in base..len {
        result += a[i] * b[i];
    }

    result
}

/// ARM NEON squared L2 distance.
#[cfg(target_arch = "aarch64")]
#[inline]
fn squared_l2_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let len = a.len();
    let simd_len = len / 4;

    let mut sum = unsafe { vdupq_n_f32(0.0) };

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..simd_len {
        let offset = i * 4;
        unsafe {
            let va = vld1q_f32(a_ptr.add(offset));
            let vb = vld1q_f32(b_ptr.add(offset));
            let diff = vsubq_f32(va, vb);
            sum = vfmaq_f32(sum, diff, diff);
        }
    }

    let mut result = unsafe { vaddvq_f32(sum) };

    let base = simd_len * 4;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}

// =============================================================================
// Public API with runtime dispatch
// =============================================================================

/// Dot product with automatic dispatch to best available SIMD.
///
/// Runtime detection selects: AVX-512 > AVX2 > NEON > Scalar
#[inline]
#[must_use]
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            return unsafe { dot_product_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") && a.len() >= 16 {
            return unsafe { dot_product_avx2(a, b) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if a.len() >= 4 {
            return dot_product_neon(a, b);
        }
    }

    // Scalar fallback
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Squared L2 distance with automatic dispatch.
#[inline]
#[must_use]
pub fn squared_l2_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") && a.len() >= 16 {
            return unsafe { squared_l2_avx512(a, b) };
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") && a.len() >= 16 {
            return unsafe { squared_l2_avx2(a, b) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if a.len() >= 4 {
            return squared_l2_neon(a, b);
        }
    }

    // Scalar fallback
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

/// Euclidean distance with automatic dispatch.
#[inline]
#[must_use]
pub fn euclidean_native(a: &[f32], b: &[f32]) -> f32 {
    squared_l2_native(a, b).sqrt()
}

/// Cosine similarity for pre-normalized vectors with automatic dispatch.
#[inline]
#[must_use]
pub fn cosine_normalized_native(a: &[f32], b: &[f32]) -> f32 {
    // For unit vectors: cos(θ) = a · b
    dot_product_native(a, b)
}

/// Full cosine similarity (with normalization) using native SIMD.
#[inline]
#[must_use]
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Compute dot product and norms in single pass for better cache utilization
    let mut dot = 0.0_f32;
    let mut norm_a_sq = 0.0_f32;
    let mut norm_b_sq = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
    }

    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Batch dot products with prefetching.
///
/// Computes dot products between a query and multiple candidates,
/// using software prefetch hints for cache optimization.
#[must_use]
pub fn batch_dot_product_native(candidates: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        // Prefetch ahead for cache warming
        #[cfg(target_arch = "x86_64")]
        if i + 4 < candidates.len() {
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                _mm_prefetch(candidates[i + 4].as_ptr().cast::<i8>(), _MM_HINT_T0);
            }
        }

        // Note: aarch64 prefetch requires unstable feature, skipped for now
        // See: https://github.com/rust-lang/rust/issues/117217

        results.push(dot_product_native(candidate, query));
    }

    results
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn test_dot_product_native_basic() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![5.0, 6.0, 7.0, 8.0];
        let result = dot_product_native(&a, &b);
        let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!((result - expected).abs() < 1e-5);
    }

    #[test]
    fn test_dot_product_native_large() {
        let a: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let b: Vec<f32> = (0..768).map(|i| (768 - i) as f32 * 0.001).collect();
        let result = dot_product_native(&a, &b);
        let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!(
            (result - expected).abs() < 0.01,
            "result={result}, expected={expected}"
        );
    }

    #[test]
    fn test_squared_l2_native_basic() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![5.0, 6.0, 7.0, 8.0];
        let result = squared_l2_native(&a, &b);
        let expected: f32 = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum();
        assert!((result - expected).abs() < 1e-5);
    }

    #[test]
    fn test_squared_l2_native_large() {
        let a: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let b: Vec<f32> = (0..768).map(|i| (768 - i) as f32 * 0.001).collect();
        let result = squared_l2_native(&a, &b);
        let expected: f32 = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum();
        assert!(
            (result - expected).abs() < 0.01,
            "result={result}, expected={expected}"
        );
    }

    #[test]
    fn test_cosine_normalized_native() {
        // Create unit vectors
        let a = vec![0.6, 0.8, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0, 0.0];
        let result = cosine_normalized_native(&a, &b);
        assert!((result - 0.6).abs() < 1e-5);
    }

    #[test]
    fn test_batch_dot_product_native() {
        let query = vec![1.0, 2.0, 3.0, 4.0];
        let candidates: Vec<Vec<f32>> = vec![
            vec![1.0, 0.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 0.0],
            vec![0.0, 0.0, 0.0, 1.0],
        ];
        let refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

        let results = batch_dot_product_native(&refs, &query);
        assert_eq!(results.len(), 4);
        assert!((results[0] - 1.0).abs() < 1e-5);
        assert!((results[1] - 2.0).abs() < 1e-5);
        assert!((results[2] - 3.0).abs() < 1e-5);
        assert!((results[3] - 4.0).abs() < 1e-5);
    }
}
