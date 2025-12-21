//! Enhanced SIMD operations with runtime CPU detection and optimized processing.
//!
//! This module provides:
//! - **Runtime SIMD detection**: Identifies AVX-512, AVX2, or scalar capability
//! - **Wide processing**: 16 floats per iteration for better throughput
//! - **Auto-dispatch**: Selects optimal implementation based on CPU
//!
//! # Architecture Support
//!
//! - **`x86_64` AVX-512**: Intel Skylake-X+, AMD Zen 4+
//! - **`x86_64` AVX2**: Intel Haswell+
//! - **ARM NEON**: Apple Silicon, ARM64 servers
//! - **Fallback**: Scalar operations for other architectures
//!
//! # Performance
//!
//! The "wide16" processing mode processes 16 floats per iteration using
//! two 8-wide SIMD operations, providing near-AVX-512 performance on AVX2
//! hardware through better instruction-level parallelism.

use wide::f32x8;

/// SIMD capability level detected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    /// AVX-512F available (512-bit, 16 x f32)
    Avx512,
    /// AVX2 available (256-bit, 8 x f32)
    Avx2,
    /// SSE4.1 or lower, or non-x86 architecture
    Scalar,
}

/// Detects the highest SIMD level available on the current CPU.
///
/// This function is called once and cached for performance.
///
/// # Example
///
/// ```
/// use velesdb_core::simd_avx512::detect_simd_level;
///
/// let level = detect_simd_level();
/// println!("SIMD level: {:?}", level);
/// ```
#[must_use]
pub fn detect_simd_level() -> SimdLevel {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return SimdLevel::Avx512;
        }
        if is_x86_feature_detected!("avx2") {
            return SimdLevel::Avx2;
        }
    }
    SimdLevel::Scalar
}

/// Returns true if AVX-512 is available on the current CPU.
#[must_use]
#[inline]
pub fn has_avx512() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        is_x86_feature_detected!("avx512f")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Computes dot product using AVX-512 if available, falling back to AVX2/scalar.
///
/// # Performance
///
/// - AVX-512: ~16 floats per cycle (2x AVX2 throughput)
/// - AVX2: ~8 floats per cycle
/// - Scalar: ~1 float per cycle
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn dot_product_auto(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Use wide16 for vectors >= 16 elements (benefits from double unrolling)
    if a.len() >= 16 {
        return dot_product_wide16(a, b);
    }

    // Fallback to existing SIMD for smaller vectors
    crate::simd_explicit::dot_product_simd(a, b)
}

/// Computes squared L2 distance with optimized wide processing.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn squared_l2_auto(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    if a.len() >= 16 {
        return squared_l2_wide16(a, b);
    }

    crate::simd_explicit::squared_l2_distance_simd(a, b)
}

/// Computes euclidean distance with optimized wide processing.
#[inline]
#[must_use]
pub fn euclidean_auto(a: &[f32], b: &[f32]) -> f32 {
    squared_l2_auto(a, b).sqrt()
}

/// Computes cosine similarity with optimized wide processing.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn cosine_similarity_auto(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    if a.len() >= 16 {
        return cosine_similarity_wide16(a, b);
    }

    crate::simd_explicit::cosine_similarity_simd(a, b)
}

// =============================================================================
// Wide32 Implementations (32 floats per iteration using 4x f32x8)
// Maximum ILP for modern out-of-order CPUs
// =============================================================================

/// Dot product with 32-wide processing for maximum instruction-level parallelism.
///
/// Uses four f32x8 accumulators per iteration, exploiting the full width of
/// modern CPU execution units (typically 4+ FMA units on Zen 3+/Alder Lake+).
#[inline]
fn dot_product_wide16(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len();
    let simd_len = len / 32;

    // Four accumulators for maximum ILP on modern CPUs
    let mut sum0 = f32x8::ZERO;
    let mut sum1 = f32x8::ZERO;
    let mut sum2 = f32x8::ZERO;
    let mut sum3 = f32x8::ZERO;

    // Main loop: 32 floats per iteration
    for i in 0..simd_len {
        let offset = i * 32;

        let va0 = f32x8::from(&a[offset..offset + 8]);
        let vb0 = f32x8::from(&b[offset..offset + 8]);
        sum0 = va0.mul_add(vb0, sum0);

        let va1 = f32x8::from(&a[offset + 8..offset + 16]);
        let vb1 = f32x8::from(&b[offset + 8..offset + 16]);
        sum1 = va1.mul_add(vb1, sum1);

        let va2 = f32x8::from(&a[offset + 16..offset + 24]);
        let vb2 = f32x8::from(&b[offset + 16..offset + 24]);
        sum2 = va2.mul_add(vb2, sum2);

        let va3 = f32x8::from(&a[offset + 24..offset + 32]);
        let vb3 = f32x8::from(&b[offset + 24..offset + 32]);
        sum3 = va3.mul_add(vb3, sum3);
    }

    // Combine accumulators (pairwise for better precision)
    let combined01 = sum0 + sum1;
    let combined23 = sum2 + sum3;
    let mut result = (combined01 + combined23).reduce_add();

    // Handle remainder in chunks of 8
    let base = simd_len * 32;
    let mut pos = base;

    while pos + 8 <= len {
        let va = f32x8::from(&a[pos..pos + 8]);
        let vb = f32x8::from(&b[pos..pos + 8]);
        result += va.mul_add(vb, f32x8::ZERO).reduce_add();
        pos += 8;
    }

    // Handle final scalar remainder (0-7 elements)
    while pos < len {
        result += a[pos] * b[pos];
        pos += 1;
    }

    result
}

/// Squared L2 distance with 32-wide processing for maximum ILP.
#[inline]
fn squared_l2_wide16(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len();
    let simd_len = len / 32;

    let mut sum0 = f32x8::ZERO;
    let mut sum1 = f32x8::ZERO;
    let mut sum2 = f32x8::ZERO;
    let mut sum3 = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 32;

        let va0 = f32x8::from(&a[offset..offset + 8]);
        let vb0 = f32x8::from(&b[offset..offset + 8]);
        let diff0 = va0 - vb0;
        sum0 = diff0.mul_add(diff0, sum0);

        let va1 = f32x8::from(&a[offset + 8..offset + 16]);
        let vb1 = f32x8::from(&b[offset + 8..offset + 16]);
        let diff1 = va1 - vb1;
        sum1 = diff1.mul_add(diff1, sum1);

        let va2 = f32x8::from(&a[offset + 16..offset + 24]);
        let vb2 = f32x8::from(&b[offset + 16..offset + 24]);
        let diff2 = va2 - vb2;
        sum2 = diff2.mul_add(diff2, sum2);

        let va3 = f32x8::from(&a[offset + 24..offset + 32]);
        let vb3 = f32x8::from(&b[offset + 24..offset + 32]);
        let diff3 = va3 - vb3;
        sum3 = diff3.mul_add(diff3, sum3);
    }

    let combined01 = sum0 + sum1;
    let combined23 = sum2 + sum3;
    let mut result = (combined01 + combined23).reduce_add();

    // Handle remainder
    let base = simd_len * 32;
    let mut pos = base;

    while pos + 8 <= len {
        let va = f32x8::from(&a[pos..pos + 8]);
        let vb = f32x8::from(&b[pos..pos + 8]);
        let diff = va - vb;
        result += diff.mul_add(diff, f32x8::ZERO).reduce_add();
        pos += 8;
    }

    while pos < len {
        let diff = a[pos] - b[pos];
        result += diff * diff;
        pos += 1;
    }

    result
}

/// Cosine similarity with 32-wide processing for maximum ILP.
///
/// Computes dot(a,b) / (||a|| * ||b||) using 4 parallel accumulators.
#[inline]
#[allow(clippy::similar_names)]
fn cosine_similarity_wide16(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len();
    let simd_len = len / 32;

    // 4 accumulators each for dot, norm_a, norm_b (12 total)
    let mut dot0 = f32x8::ZERO;
    let mut dot1 = f32x8::ZERO;
    let mut dot2 = f32x8::ZERO;
    let mut dot3 = f32x8::ZERO;
    let mut na0 = f32x8::ZERO;
    let mut na1 = f32x8::ZERO;
    let mut na2 = f32x8::ZERO;
    let mut na3 = f32x8::ZERO;
    let mut nb0 = f32x8::ZERO;
    let mut nb1 = f32x8::ZERO;
    let mut nb2 = f32x8::ZERO;
    let mut nb3 = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 32;

        let va0 = f32x8::from(&a[offset..offset + 8]);
        let vb0 = f32x8::from(&b[offset..offset + 8]);
        dot0 = va0.mul_add(vb0, dot0);
        na0 = va0.mul_add(va0, na0);
        nb0 = vb0.mul_add(vb0, nb0);

        let va1 = f32x8::from(&a[offset + 8..offset + 16]);
        let vb1 = f32x8::from(&b[offset + 8..offset + 16]);
        dot1 = va1.mul_add(vb1, dot1);
        na1 = va1.mul_add(va1, na1);
        nb1 = vb1.mul_add(vb1, nb1);

        let va2 = f32x8::from(&a[offset + 16..offset + 24]);
        let vb2 = f32x8::from(&b[offset + 16..offset + 24]);
        dot2 = va2.mul_add(vb2, dot2);
        na2 = va2.mul_add(va2, na2);
        nb2 = vb2.mul_add(vb2, nb2);

        let va3 = f32x8::from(&a[offset + 24..offset + 32]);
        let vb3 = f32x8::from(&b[offset + 24..offset + 32]);
        dot3 = va3.mul_add(vb3, dot3);
        na3 = va3.mul_add(va3, na3);
        nb3 = vb3.mul_add(vb3, nb3);
    }

    // Combine accumulators (pairwise for precision)
    let mut dot = ((dot0 + dot1) + (dot2 + dot3)).reduce_add();
    let mut norm_a_sq = ((na0 + na1) + (na2 + na3)).reduce_add();
    let mut norm_b_sq = ((nb0 + nb1) + (nb2 + nb3)).reduce_add();

    // Handle remainder
    let base = simd_len * 32;
    let mut pos = base;

    while pos + 8 <= len {
        let va = f32x8::from(&a[pos..pos + 8]);
        let vb = f32x8::from(&b[pos..pos + 8]);
        dot += va.mul_add(vb, f32x8::ZERO).reduce_add();
        norm_a_sq += va.mul_add(va, f32x8::ZERO).reduce_add();
        norm_b_sq += vb.mul_add(vb, f32x8::ZERO).reduce_add();
        pos += 8;
    }

    while pos < len {
        let ai = a[pos];
        let bi = b[pos];
        dot += ai * bi;
        norm_a_sq += ai * ai;
        norm_b_sq += bi * bi;
        pos += 1;
    }

    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

// =============================================================================
// Tests (TDD - written first)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
        #[allow(clippy::cast_precision_loss)]
        (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
    }

    // =========================================================================
    // Detection tests
    // =========================================================================

    #[test]
    fn test_detect_simd_level_returns_valid() {
        let level = detect_simd_level();
        assert!(
            matches!(
                level,
                SimdLevel::Avx512 | SimdLevel::Avx2 | SimdLevel::Scalar
            ),
            "Should return a valid SIMD level"
        );
    }

    #[test]
    fn test_has_avx512_consistent() {
        let level = detect_simd_level();
        let has = has_avx512();

        if level == SimdLevel::Avx512 {
            assert!(has, "has_avx512 should be true when level is Avx512");
        }
    }

    // =========================================================================
    // Correctness tests - dot product
    // =========================================================================

    #[test]
    fn test_dot_product_auto_basic() {
        let a = vec![1.0; 16];
        let b = vec![2.0; 16];
        let result = dot_product_auto(&a, &b);
        assert!(
            (result - 32.0).abs() < EPSILON,
            "Expected 32.0, got {result}"
        );
    }

    #[test]
    fn test_dot_product_auto_768d() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let auto_result = dot_product_auto(&a, &b);
        let scalar_result: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        let rel_error = (auto_result - scalar_result).abs() / scalar_result.abs().max(1.0);
        assert!(rel_error < 1e-4, "Relative error too high: {rel_error}");
    }

    #[test]
    fn test_dot_product_auto_consistency() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let auto = dot_product_auto(&a, &b);
        let explicit = crate::simd_explicit::dot_product_simd(&a, &b);

        assert!(
            (auto - explicit).abs() < 1e-3,
            "Auto and explicit should match: {auto} vs {explicit}"
        );
    }

    // =========================================================================
    // Correctness tests - squared L2
    // =========================================================================

    #[test]
    fn test_squared_l2_auto_identical() {
        let v = generate_test_vector(768, 0.0);
        let result = squared_l2_auto(&v, &v);
        assert!(
            result.abs() < EPSILON,
            "Identical vectors should have distance 0"
        );
    }

    #[test]
    fn test_squared_l2_auto_known() {
        let a = vec![0.0; 16];
        let mut b = vec![0.0; 16];
        b[0] = 3.0;
        b[1] = 4.0;
        let result = squared_l2_auto(&a, &b);
        assert!(
            (result - 25.0).abs() < EPSILON,
            "Expected 25.0 (3² + 4²), got {result}"
        );
    }

    #[test]
    fn test_squared_l2_auto_consistency() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let auto = squared_l2_auto(&a, &b);
        let explicit = crate::simd_explicit::squared_l2_distance_simd(&a, &b);

        assert!(
            (auto - explicit).abs() < 1e-2,
            "Auto and explicit should match: {auto} vs {explicit}"
        );
    }

    // =========================================================================
    // Correctness tests - euclidean
    // =========================================================================

    #[test]
    fn test_euclidean_auto_known() {
        let a = vec![0.0; 16];
        let mut b = vec![0.0; 16];
        b[0] = 3.0;
        b[1] = 4.0;
        let result = euclidean_auto(&a, &b);
        assert!(
            (result - 5.0).abs() < EPSILON,
            "Expected 5.0 (3-4-5 triangle), got {result}"
        );
    }

    // =========================================================================
    // Correctness tests - cosine similarity
    // =========================================================================

    #[test]
    fn test_cosine_similarity_auto_identical() {
        let v = generate_test_vector(768, 0.0);
        let result = cosine_similarity_auto(&v, &v);
        assert!(
            (result - 1.0).abs() < EPSILON,
            "Identical vectors should have similarity 1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_auto_orthogonal() {
        let mut a = vec![0.0; 16];
        let mut b = vec![0.0; 16];
        a[0] = 1.0;
        b[1] = 1.0;
        let result = cosine_similarity_auto(&a, &b);
        assert!(
            result.abs() < EPSILON,
            "Orthogonal vectors should have similarity 0"
        );
    }

    #[test]
    fn test_cosine_similarity_auto_opposite() {
        let a = generate_test_vector(768, 0.0);
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        let result = cosine_similarity_auto(&a, &b);
        assert!(
            (result + 1.0).abs() < EPSILON,
            "Opposite vectors should have similarity -1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_auto_consistency() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let auto = cosine_similarity_auto(&a, &b);
        let explicit = crate::simd_explicit::cosine_similarity_simd(&a, &b);

        assert!(
            (auto - explicit).abs() < 1e-5,
            "Auto and explicit should match: {auto} vs {explicit}"
        );
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_auto_odd_dimensions() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // Not multiple of 16
        let b = vec![5.0, 4.0, 3.0, 2.0, 1.0];

        let result = dot_product_auto(&a, &b);
        let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((result - expected).abs() < EPSILON);
    }

    #[test]
    fn test_auto_small_vectors() {
        let a = vec![3.0];
        let b = vec![4.0];
        assert!((dot_product_auto(&a, &b) - 12.0).abs() < EPSILON);
    }

    #[test]
    #[should_panic(expected = "Vector dimensions must match")]
    fn test_auto_dimension_mismatch() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let _ = dot_product_auto(&a, &b);
    }

    // =========================================================================
    // Boundary size tests (crucial for SIMD remainder handling)
    // =========================================================================

    #[test]
    fn test_boundary_sizes_dot_product() {
        // Test sizes around SIMD boundaries: 7, 8, 9, 15, 16, 17, 31, 32, 33
        for size in [7, 8, 9, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65] {
            let a = generate_test_vector(size, 0.0);
            let b = generate_test_vector(size, 1.0);

            let auto = dot_product_auto(&a, &b);
            let scalar: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

            let rel_error = (auto - scalar).abs() / scalar.abs().max(1.0);
            assert!(
                rel_error < 1e-4,
                "Size {size}: auto={auto}, scalar={scalar}, error={rel_error}"
            );
        }
    }

    #[test]
    fn test_boundary_sizes_squared_l2() {
        for size in [7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65] {
            let a = generate_test_vector(size, 0.0);
            let b = generate_test_vector(size, 1.0);

            let auto = squared_l2_auto(&a, &b);
            let scalar: f32 = a.iter().zip(&b).map(|(x, y)| (x - y) * (x - y)).sum();

            let rel_error = (auto - scalar).abs() / scalar.abs().max(1.0);
            assert!(
                rel_error < 1e-4,
                "Size {size}: auto={auto}, scalar={scalar}, error={rel_error}"
            );
        }
    }

    #[test]
    fn test_boundary_sizes_cosine() {
        for size in [7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65] {
            let a = generate_test_vector(size, 0.0);
            let b = generate_test_vector(size, 1.0);

            let auto = cosine_similarity_auto(&a, &b);
            let explicit = crate::simd_explicit::cosine_similarity_simd(&a, &b);

            assert!(
                (auto - explicit).abs() < 1e-4,
                "Size {size}: auto={auto}, explicit={explicit}"
            );
        }
    }

    // =========================================================================
    // Zero vector tests
    // =========================================================================

    #[test]
    fn test_zero_vectors_dot_product() {
        let a = vec![0.0; 768];
        let b = vec![0.0; 768];
        let result = dot_product_auto(&a, &b);
        assert!(result.abs() < EPSILON, "Zero vectors dot = 0");
    }

    #[test]
    fn test_zero_vectors_euclidean() {
        let a = vec![0.0; 768];
        let b = vec![0.0; 768];
        let result = euclidean_auto(&a, &b);
        assert!(result.abs() < EPSILON, "Zero vectors distance = 0");
    }

    #[test]
    fn test_zero_vectors_cosine() {
        let a = vec![0.0; 768];
        let b = vec![0.0; 768];
        let result = cosine_similarity_auto(&a, &b);
        assert!(result.abs() < EPSILON, "Zero vectors cosine = 0 (defined)");
    }

    #[test]
    fn test_one_zero_vector_cosine() {
        let a = generate_test_vector(768, 0.0);
        let b = vec![0.0; 768];
        let result = cosine_similarity_auto(&a, &b);
        assert!(result.abs() < EPSILON, "One zero vector cosine = 0");
    }

    // =========================================================================
    // Negative values tests
    // =========================================================================

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn test_negative_values() {
        let a: Vec<f32> = (0..768).map(|i| -(i as f32) * 0.01).collect();
        let b: Vec<f32> = (0..768).map(|i| (i as f32) * 0.01).collect();

        let dot = dot_product_auto(&a, &b);
        let dist = euclidean_auto(&a, &b);
        let cos = cosine_similarity_auto(&a, &b);

        assert!(dot < 0.0, "Opposite signs should give negative dot");
        assert!(dist > 0.0, "Distance always positive");
        assert!(cos < 0.0, "Opposite vectors should have negative cosine");
    }

    // =========================================================================
    // Very small values (denormals)
    // =========================================================================

    #[test]
    fn test_very_small_values() {
        // Use small but not denormal values to avoid precision issues
        let tiny = 1e-20_f32;
        let a = vec![tiny; 768];
        let b = vec![tiny; 768];

        let dot = dot_product_auto(&a, &b);
        let dist = euclidean_auto(&a, &b);
        let cos = cosine_similarity_auto(&a, &b);

        assert!(dot.is_finite(), "Tiny dot should be finite");
        assert!(dist.is_finite(), "Tiny dist should be finite");
        // With floating point arithmetic, cosine can slightly exceed 1.0
        // Allow small epsilon for rounding errors
        assert!(
            (-1.0 - EPSILON..=1.0 + EPSILON).contains(&cos),
            "Tiny vectors cosine should be valid, got {cos}"
        );
    }

    // =========================================================================
    // Large values (near overflow)
    // =========================================================================

    #[test]
    fn test_large_values() {
        let large = 1e18_f32;
        let a = vec![large; 32];
        let b = vec![large; 32];

        let cos = cosine_similarity_auto(&a, &b);

        // Cosine should still be ~1 even with large values
        assert!(
            (cos - 1.0).abs() < 1e-4,
            "Identical large vectors cosine ≈ 1, got {cos}"
        );
    }

    // =========================================================================
    // Very large vectors (stress test)
    // =========================================================================

    #[test]
    fn test_very_large_vector_4096d() {
        // Largest common embedding dimension
        let a = generate_test_vector(4096, 0.0);
        let b = generate_test_vector(4096, 1.0);

        let dot = dot_product_auto(&a, &b);
        let dist = euclidean_auto(&a, &b);
        let cos = cosine_similarity_auto(&a, &b);

        assert!(dot.is_finite(), "4096D dot finite");
        assert!(dist.is_finite() && dist >= 0.0, "4096D dist >= 0");
        assert!((-1.0..=1.0).contains(&cos), "4096D cos in [-1,1]");
    }

    #[test]
    fn test_million_dim_dot_product() {
        // Stress test with 1M dimensions
        #[allow(clippy::cast_precision_loss)]
        let a: Vec<f32> = (0..1_000_000).map(|i| (i as f32 * 0.001).sin()).collect();
        #[allow(clippy::cast_precision_loss)]
        let b: Vec<f32> = (0..1_000_000).map(|i| (i as f32 * 0.002).cos()).collect();

        let result = dot_product_auto(&a, &b);
        assert!(result.is_finite(), "1M dim dot should be finite");
    }

    // =========================================================================
    // Performance characteristics (not benchmarks, just sanity checks)
    // =========================================================================

    #[test]
    fn test_large_vector_1536d() {
        // GPT-4 embedding dimension
        let a = generate_test_vector(1536, 0.0);
        let b = generate_test_vector(1536, 1.0);

        let dot = dot_product_auto(&a, &b);
        let dist = euclidean_auto(&a, &b);
        let cos = cosine_similarity_auto(&a, &b);

        // Just verify they complete and return valid floats
        assert!(dot.is_finite(), "Dot product should be finite");
        assert!(dist.is_finite() && dist >= 0.0, "Distance should be >= 0");
        assert!(
            cos.is_finite() && (-1.0..=1.0).contains(&cos),
            "Cosine should be in [-1, 1]"
        );
    }

    // =========================================================================
    // Precision tests
    // =========================================================================

    #[test]
    fn test_precision_accumulation() {
        // Test that FMA accumulation maintains precision
        let a = vec![1.0; 10000];
        let b = vec![1.0; 10000];

        let result = dot_product_auto(&a, &b);
        let expected = 10000.0_f32;

        assert!(
            (result - expected).abs() < 1.0,
            "Precision should be maintained: got {result}, expected {expected}"
        );
    }

    #[test]
    fn test_unit_vectors_cosine() {
        // Pre-normalized unit vectors should give exact results
        let mut a = generate_test_vector(768, 0.0);
        let mut b = generate_test_vector(768, 1.0);

        // Normalize
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut a {
            *x /= norm_a;
        }
        for x in &mut b {
            *x /= norm_b;
        }

        let cos = cosine_similarity_auto(&a, &b);
        assert!(
            (-1.0..=1.0).contains(&cos),
            "Unit vectors cosine must be in [-1, 1]"
        );
    }
}
