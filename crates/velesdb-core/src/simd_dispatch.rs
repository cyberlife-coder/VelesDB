//! Zero-overhead SIMD function dispatch.
//!
//! This module provides a thin wrapper around `simd_native` functions,
//! offering a stable public API while `simd_native` handles the
//! architecture-specific SIMD implementations internally.
//!
//! # EPIC-C.2: TS-SIMD-002

// SAFETY: Numeric casts in SIMD dispatch are intentional:
// - usize->u32 for Hamming distance: vector dimensions bounded by implementation
// - Maximum dimension is 65536, result fits in u32
#![allow(clippy::cast_possible_truncation)]

// =============================================================================
// Public dispatch API - Direct calls to simd_native
// =============================================================================

/// Compute dot product with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn dot_product_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::dot_product_native(a, b)
}

/// Compute Euclidean distance with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn euclidean_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::euclidean_native(a, b)
}

/// Compute cosine similarity with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn cosine_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::cosine_similarity_native(a, b)
}

/// Compute cosine similarity for pre-normalized vectors.
#[inline]
#[must_use]
pub fn cosine_normalized_dispatched(a: &[f32], b: &[f32]) -> f32 {
    crate::simd_native::cosine_normalized_native(a, b)
}

/// Compute Hamming distance with automatic SIMD dispatch.
#[inline]
#[must_use]
pub fn hamming_dispatched(a: &[f32], b: &[f32]) -> u32 {
    #[allow(clippy::cast_sign_loss)]
    // SAFETY: hamming_distance_native returns count of differing bits (non-negative),
    // and vector dimensions are bounded by u32::MAX, so result always fits in u32
    {
        crate::simd_native::hamming_distance_native(a, b) as u32
    }
}

/// Returns information about which SIMD features are available.
#[must_use]
pub fn simd_features_info() -> SimdFeatures {
    SimdFeatures::detect()
}

/// Information about available SIMD features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SimdFeatures {
    /// AVX-512 foundation instructions available.
    pub avx512f: bool,
    /// AVX-512 VPOPCNTDQ (population count) available.
    pub avx512_popcnt: bool,
    /// AVX2 instructions available.
    pub avx2: bool,
    /// POPCNT instruction available.
    pub popcnt: bool,
}

impl SimdFeatures {
    /// Detects available SIMD features on the current CPU.
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self {
                avx512f: is_x86_feature_detected!("avx512f"),
                avx512_popcnt: is_x86_feature_detected!("avx512vpopcntdq"),
                avx2: is_x86_feature_detected!("avx2"),
                popcnt: is_x86_feature_detected!("popcnt"),
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                avx512f: false,
                avx512_popcnt: false,
                avx2: false,
                popcnt: false,
            }
        }
    }

    /// Returns the best available instruction set name.
    #[must_use]
    pub const fn best_instruction_set(&self) -> &'static str {
        if self.avx512f {
            "AVX-512"
        } else if self.avx2 {
            "AVX2"
        } else {
            "Scalar"
        }
    }
}

// =============================================================================
// Prefetch constants - EPIC-C.1
// =============================================================================

// Scalar implementations for tests
#[cfg(test)]
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
fn euclidean_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum::<f32>()
        .sqrt()
}

#[cfg(test)]
fn cosine_scalar(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom > 0.0 {
        dot / denom
    } else {
        0.0
    }
}

#[cfg(test)]
fn hamming_scalar(a: &[f32], b: &[f32]) -> u32 {
    assert_eq!(a.len(), b.len(), "Vector length mismatch");
    #[allow(clippy::cast_possible_truncation)]
    let count = a
        .iter()
        .zip(b.iter())
        .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
        .count() as u32;
    count
}

#[cfg(test)]
fn cosine_normalized_scalar(a: &[f32], b: &[f32]) -> f32 {
    // For normalized vectors, cosine = dot product
    dot_product_scalar(a, b)
}

/// Cache line size in bytes (standard for modern x86/ARM CPUs).
pub const CACHE_LINE_SIZE: usize = 64;

/// Prefetch distance for 768-dimensional vectors (3072 bytes).
/// Calculated at compile time: `768 * 4 / 64 = 48` cache lines.
pub const PREFETCH_DISTANCE_768D: usize = 768 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Prefetch distance for 384-dimensional vectors.
pub const PREFETCH_DISTANCE_384D: usize = 384 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Prefetch distance for 1536-dimensional vectors.
pub const PREFETCH_DISTANCE_1536D: usize = 1536 * std::mem::size_of::<f32>() / CACHE_LINE_SIZE;

/// Calculates prefetch distance for a given dimension at compile time.
#[inline]
#[must_use]
pub const fn prefetch_distance(dimension: usize) -> usize {
    (dimension * std::mem::size_of::<f32>()) / CACHE_LINE_SIZE
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Dispatched Function Tests
    // =========================================================================

    #[test]
    fn test_dot_product_dispatched_basic() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![5.0, 6.0, 7.0, 8.0];
        let result = dot_product_dispatched(&a, &b);
        // 1*5 + 2*6 + 3*7 + 4*8 = 70
        assert!((result - 70.0).abs() < 1e-5);
    }

    #[test]
    fn test_dot_product_dispatched_large() {
        let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.001).sin()).collect();
        let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.001).cos()).collect();
        let result = dot_product_dispatched(&a, &b);
        assert!(result.is_finite());
    }

    #[test]
    fn test_euclidean_dispatched_basic() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        let result = euclidean_dispatched(&a, &b);
        assert!((result - 5.0).abs() < 1e-5);
    }

    #[test]
    fn test_euclidean_dispatched_identical() {
        let a: Vec<f32> = vec![1.0; 64];
        let result = euclidean_dispatched(&a, &a);
        assert!(result.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_dispatched_identical() {
        let a: Vec<f32> = vec![1.0; 32];
        let result = cosine_dispatched(&a, &a);
        assert!((result - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_dispatched_orthogonal() {
        let mut a = vec![0.0; 32];
        let mut b = vec![0.0; 32];
        a[0] = 1.0;
        b[1] = 1.0;
        let result = cosine_dispatched(&a, &b);
        assert!(result.abs() < 1e-5);
    }

    #[test]
    fn test_cosine_dispatched_opposite() {
        let a: Vec<f32> = vec![1.0; 16];
        let b: Vec<f32> = vec![-1.0; 16];
        let result = cosine_dispatched(&a, &b);
        assert!((result - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_normalized_dispatched() {
        // Pre-normalized unit vectors
        let norm = (32.0_f32).sqrt();
        let a: Vec<f32> = vec![1.0 / norm; 32];
        let result = cosine_normalized_dispatched(&a, &a);
        assert!((result - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_hamming_dispatched_identical() {
        let a: Vec<f32> = vec![1.0; 32];
        let result = hamming_dispatched(&a, &a);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_hamming_dispatched_different() {
        let a: Vec<f32> = vec![1.0; 32]; // All above 0.5
        let b: Vec<f32> = vec![0.0; 32]; // All below 0.5
        let result = hamming_dispatched(&a, &b);
        assert_eq!(result, 32);
    }

    #[test]
    fn test_hamming_dispatched_half() {
        let a = vec![1.0; 32];
        let mut b = vec![1.0; 32];
        // Make half different
        for item in b.iter_mut().take(16) {
            *item = 0.0;
        }
        let result = hamming_dispatched(&a, &b);
        assert_eq!(result, 16);
    }

    // =========================================================================
    // SimdFeatures Tests
    // =========================================================================

    #[test]
    fn test_simd_features_detect() {
        let features = SimdFeatures::detect();
        // Just verify detection doesn't panic
        let _ = features.avx512f;
        let _ = features.avx2;
        let _ = features.popcnt;
    }

    #[test]
    fn test_simd_features_info() {
        let features = simd_features_info();
        // Verify struct fields are accessible
        let _ = features.avx512f;
    }

    #[test]
    fn test_simd_features_best_instruction_set() {
        let features = SimdFeatures::detect();
        let best = features.best_instruction_set();
        assert!(
            best == "AVX-512" || best == "AVX2" || best == "Scalar",
            "Unexpected instruction set: {best}"
        );
    }

    #[test]
    fn test_simd_features_debug() {
        let features = SimdFeatures::detect();
        let debug_str = format!("{:?}", features);
        assert!(debug_str.contains("SimdFeatures"));
    }

    #[test]
    fn test_simd_features_clone() {
        let features = SimdFeatures::detect();
        let cloned = features;
        assert_eq!(features, cloned);
    }

    // =========================================================================
    // Scalar Fallback Tests
    // =========================================================================

    #[test]
    fn test_dot_product_scalar() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let result = dot_product_scalar(&a, &b);
        // 1*4 + 2*5 + 3*6 = 32
        assert!((result - 32.0).abs() < 1e-6);
    }

    #[test]
    fn test_euclidean_scalar() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 2.0];
        let result = euclidean_scalar(&a, &b);
        // sqrt(1 + 4 + 4) = 3
        assert!((result - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_scalar_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let result = cosine_scalar(&a, &a);
        assert!((result - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_scalar_zero_norm() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let result = cosine_scalar(&a, &b);
        assert!((result - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_normalized_scalar() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let result = cosine_normalized_scalar(&a, &b);
        assert!(result.abs() < 1e-6);
    }

    #[test]
    fn test_hamming_scalar() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 1.0, 1.0, 0.0];
        let result = hamming_scalar(&a, &b);
        // Position 0: 1.0 > 0.5, 0.0 < 0.5 -> different
        // Position 1: 0.0 < 0.5, 1.0 > 0.5 -> different
        // Position 2: same
        // Position 3: same
        assert_eq!(result, 2);
    }

    // =========================================================================
    // Prefetch Distance Tests
    // =========================================================================

    #[test]
    fn test_prefetch_distance_384d() {
        let dist = prefetch_distance(384);
        assert_eq!(dist, PREFETCH_DISTANCE_384D);
        assert_eq!(dist, 24); // 384 * 4 / 64
    }

    #[test]
    fn test_prefetch_distance_768d() {
        let dist = prefetch_distance(768);
        assert_eq!(dist, PREFETCH_DISTANCE_768D);
        assert_eq!(dist, 48); // 768 * 4 / 64
    }

    #[test]
    fn test_prefetch_distance_1536d() {
        let dist = prefetch_distance(1536);
        assert_eq!(dist, PREFETCH_DISTANCE_1536D);
        assert_eq!(dist, 96); // 1536 * 4 / 64
    }

    #[test]
    fn test_cache_line_size() {
        assert_eq!(CACHE_LINE_SIZE, 64);
    }

    // =========================================================================
    // Edge Cases
    // =========================================================================

    #[test]
    #[should_panic(expected = "Vector length mismatch")]
    fn test_dot_product_scalar_length_mismatch() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        dot_product_scalar(&a, &b);
    }

    #[test]
    #[should_panic(expected = "Vector length mismatch")]
    fn test_euclidean_scalar_length_mismatch() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        euclidean_scalar(&a, &b);
    }

    #[test]
    #[should_panic(expected = "Vector length mismatch")]
    fn test_cosine_scalar_length_mismatch() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        cosine_scalar(&a, &b);
    }

    #[test]
    #[should_panic(expected = "Vector length mismatch")]
    fn test_hamming_scalar_length_mismatch() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        hamming_scalar(&a, &b);
    }

    #[test]
    fn test_empty_vectors() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        let dot = dot_product_scalar(&a, &b);
        assert!((dot - 0.0).abs() < 1e-6);
    }
}
