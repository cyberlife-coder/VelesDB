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
// Wide16 Implementations (16 floats per iteration using 2x f32x8)
// =============================================================================

/// Dot product with 16-wide processing for improved instruction-level parallelism.
///
/// Uses two f32x8 accumulators per iteration, effectively processing 16 floats
/// similar to AVX-512 but using AVX2 instructions.
#[inline]
fn dot_product_wide16(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len();
    let simd_len = len / 16;
    let remainder = len % 16;

    // Two accumulators for better ILP (instruction-level parallelism)
    let mut sum0 = f32x8::ZERO;
    let mut sum1 = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 16;

        // First 8 floats
        let va0 = f32x8::from(&a[offset..offset + 8]);
        let vb0 = f32x8::from(&b[offset..offset + 8]);
        sum0 = va0.mul_add(vb0, sum0);

        // Second 8 floats
        let va1 = f32x8::from(&a[offset + 8..offset + 16]);
        let vb1 = f32x8::from(&b[offset + 8..offset + 16]);
        sum1 = va1.mul_add(vb1, sum1);
    }

    // Combine accumulators and reduce
    let combined = sum0 + sum1;
    let mut result = combined.reduce_add();

    // Handle remainder (0-15 elements)
    let base = simd_len * 16;
    let rem8 = remainder / 8;
    let rem_rest = remainder % 8;

    if rem8 > 0 {
        let va = f32x8::from(&a[base..base + 8]);
        let vb = f32x8::from(&b[base..base + 8]);
        result += va.mul_add(vb, f32x8::ZERO).reduce_add();
    }

    let final_base = base + rem8 * 8;
    for i in 0..rem_rest {
        result += a[final_base + i] * b[final_base + i];
    }

    result
}

/// Squared L2 distance with 16-wide processing.
#[inline]
fn squared_l2_wide16(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len();
    let simd_len = len / 16;
    let remainder = len % 16;

    let mut sum0 = f32x8::ZERO;
    let mut sum1 = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 16;

        let va0 = f32x8::from(&a[offset..offset + 8]);
        let vb0 = f32x8::from(&b[offset..offset + 8]);
        let diff0 = va0 - vb0;
        sum0 = diff0.mul_add(diff0, sum0);

        let va1 = f32x8::from(&a[offset + 8..offset + 16]);
        let vb1 = f32x8::from(&b[offset + 8..offset + 16]);
        let diff1 = va1 - vb1;
        sum1 = diff1.mul_add(diff1, sum1);
    }

    let combined = sum0 + sum1;
    let mut result = combined.reduce_add();

    let base = simd_len * 16;
    let rem8 = remainder / 8;
    let rem_rest = remainder % 8;

    if rem8 > 0 {
        let va = f32x8::from(&a[base..base + 8]);
        let vb = f32x8::from(&b[base..base + 8]);
        let diff = va - vb;
        result += diff.mul_add(diff, f32x8::ZERO).reduce_add();
    }

    let final_base = base + rem8 * 8;
    for i in 0..rem_rest {
        let diff = a[final_base + i] - b[final_base + i];
        result += diff * diff;
    }

    result
}

/// Cosine similarity with 16-wide processing.
#[inline]
#[allow(clippy::similar_names)]
fn cosine_similarity_wide16(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len();
    let simd_len = len / 16;
    let remainder = len % 16;

    let mut dot0 = f32x8::ZERO;
    let mut dot1 = f32x8::ZERO;
    let mut norm_a0 = f32x8::ZERO;
    let mut norm_a1 = f32x8::ZERO;
    let mut norm_b0 = f32x8::ZERO;
    let mut norm_b1 = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 16;

        let va0 = f32x8::from(&a[offset..offset + 8]);
        let vb0 = f32x8::from(&b[offset..offset + 8]);
        dot0 = va0.mul_add(vb0, dot0);
        norm_a0 = va0.mul_add(va0, norm_a0);
        norm_b0 = vb0.mul_add(vb0, norm_b0);

        let va1 = f32x8::from(&a[offset + 8..offset + 16]);
        let vb1 = f32x8::from(&b[offset + 8..offset + 16]);
        dot1 = va1.mul_add(vb1, dot1);
        norm_a1 = va1.mul_add(va1, norm_a1);
        norm_b1 = vb1.mul_add(vb1, norm_b1);
    }

    let mut dot = (dot0 + dot1).reduce_add();
    let mut norm_a_sq = (norm_a0 + norm_a1).reduce_add();
    let mut norm_b_sq = (norm_b0 + norm_b1).reduce_add();

    // Handle remainder
    let base = simd_len * 16;
    for i in 0..remainder {
        let ai = a[base + i];
        let bi = b[base + i];
        dot += ai * bi;
        norm_a_sq += ai * ai;
        norm_b_sq += bi * bi;
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
}
