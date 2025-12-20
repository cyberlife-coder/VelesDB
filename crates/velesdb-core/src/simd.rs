//! SIMD-optimized vector operations for high-performance distance calculations.
//!
//! # Performance Goals (TDD)
//!
//! - `cosine_similarity_simd`: Target < 300ns for 768d (vs 755ns baseline)
//! - `euclidean_distance_simd`: Target < 150ns for 768d (vs 256ns baseline)
//! - `normalize_inplace`: Zero allocation normalization
//!
//! # Implementation Strategy
//!
//! Uses portable SIMD via manual loop unrolling and compiler auto-vectorization hints.
//! For explicit SIMD, consider `std::simd` (nightly) or `packed_simd2` crate.
//!
//! # Note on `hnsw_rs` Integration
//!
//! Custom `Distance` trait implementations for `hnsw_rs` are NOT supported due to
//! undocumented internal invariants in the library. The SIMD functions in this module
//! are used by `DistanceMetric::calculate()` for direct distance computations outside
//! of the HNSW index.

/// Computes cosine similarity using a single-pass fused algorithm.
///
/// # Algorithm
///
/// Computes dot(a,b), norm(a)², norm(b)² in a single pass to improve cache locality.
/// Result: `dot / (sqrt(norm_a) * sqrt(norm_b))`
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
#[allow(clippy::similar_names)]
pub fn cosine_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let (dot, norm_a_sq, norm_b_sq) = fused_dot_norms(a, b);

    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Computes euclidean distance with loop unrolling for better vectorization.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn euclidean_distance_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    squared_l2_distance(a, b).sqrt()
}

/// Computes squared L2 distance (avoids sqrt for comparison purposes).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn squared_l2_distance(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Process in chunks of 4 for better auto-vectorization
    let chunks = a.len() / 4;
    let remainder = a.len() % 4;

    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;

    for i in 0..chunks {
        let base = i * 4;
        let d0 = a[base] - b[base];
        let d1 = a[base + 1] - b[base + 1];
        let d2 = a[base + 2] - b[base + 2];
        let d3 = a[base + 3] - b[base + 3];

        sum0 += d0 * d0;
        sum1 += d1 * d1;
        sum2 += d2 * d2;
        sum3 += d3 * d3;
    }

    // Handle remainder
    let base = chunks * 4;
    for i in 0..remainder {
        let d = a[base + i] - b[base + i];
        sum0 += d * d;
    }

    sum0 + sum1 + sum2 + sum3
}

/// Computes dot product and both squared norms in a single pass.
///
/// Returns (`dot_product`, `norm_a_squared`, `norm_b_squared`)
#[inline]
#[allow(clippy::similar_names)]
fn fused_dot_norms(a: &[f32], b: &[f32]) -> (f32, f32, f32) {
    let chunks = a.len() / 4;
    let remainder = a.len() % 4;

    let mut dot0 = 0.0f32;
    let mut dot1 = 0.0f32;
    let mut dot2 = 0.0f32;
    let mut dot3 = 0.0f32;

    let mut norm_a0 = 0.0f32;
    let mut norm_a1 = 0.0f32;
    let mut norm_a2 = 0.0f32;
    let mut norm_a3 = 0.0f32;

    let mut norm_b0 = 0.0f32;
    let mut norm_b1 = 0.0f32;
    let mut norm_b2 = 0.0f32;
    let mut norm_b3 = 0.0f32;

    for i in 0..chunks {
        let base = i * 4;

        let a0 = a[base];
        let a1 = a[base + 1];
        let a2 = a[base + 2];
        let a3 = a[base + 3];

        let b0 = b[base];
        let b1 = b[base + 1];
        let b2 = b[base + 2];
        let b3 = b[base + 3];

        dot0 += a0 * b0;
        dot1 += a1 * b1;
        dot2 += a2 * b2;
        dot3 += a3 * b3;

        norm_a0 += a0 * a0;
        norm_a1 += a1 * a1;
        norm_a2 += a2 * a2;
        norm_a3 += a3 * a3;

        norm_b0 += b0 * b0;
        norm_b1 += b1 * b1;
        norm_b2 += b2 * b2;
        norm_b3 += b3 * b3;
    }

    // Handle remainder
    let base = chunks * 4;
    for i in 0..remainder {
        let ai = a[base + i];
        let bi = b[base + i];
        dot0 += ai * bi;
        norm_a0 += ai * ai;
        norm_b0 += bi * bi;
    }

    (
        dot0 + dot1 + dot2 + dot3,
        norm_a0 + norm_a1 + norm_a2 + norm_a3,
        norm_b0 + norm_b1 + norm_b2 + norm_b3,
    )
}

/// Normalizes a vector in-place (zero allocation).
///
/// # Panics
///
/// Does not panic on zero vector (leaves unchanged).
#[inline]
pub fn normalize_inplace(v: &mut [f32]) {
    let norm_sq: f32 = v.iter().map(|x| x * x).sum();

    if norm_sq == 0.0 {
        return;
    }

    let inv_norm = 1.0 / norm_sq.sqrt();

    // Unrolled for auto-vectorization
    let chunks = v.len() / 4;
    let remainder = v.len() % 4;

    for i in 0..chunks {
        let base = i * 4;
        v[base] *= inv_norm;
        v[base + 1] *= inv_norm;
        v[base + 2] *= inv_norm;
        v[base + 3] *= inv_norm;
    }

    let base = chunks * 4;
    for i in 0..remainder {
        v[base + i] *= inv_norm;
    }
}

/// Computes the L2 norm (magnitude) of a vector.
#[inline]
#[must_use]
pub fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Computes dot product with loop unrolling.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn dot_product_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let chunks = a.len() / 4;
    let remainder = a.len() % 4;

    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;

    for i in 0..chunks {
        let base = i * 4;
        sum0 += a[base] * b[base];
        sum1 += a[base + 1] * b[base + 1];
        sum2 += a[base + 2] * b[base + 2];
        sum3 += a[base + 3] * b[base + 3];
    }

    let base = chunks * 4;
    for i in 0..remainder {
        sum0 += a[base + i] * b[base + i];
    }

    sum0 + sum1 + sum2 + sum3
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
#[inline]
#[must_use]
pub fn jaccard_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let mut intersection = 0u32;
    let mut union = 0u32;

    for i in 0..a.len() {
        let in_a = a[i] > 0.5;
        let in_b = b[i] > 0.5;

        if in_a && in_b {
            intersection += 1;
        }
        if in_a || in_b {
            union += 1;
        }
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

#[cfg(test)]
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
}
