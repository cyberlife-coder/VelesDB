//! Tests for `simd_explicit` module

#[cfg(test)]
mod tests {
    use crate::simd_explicit::*;

    const EPSILON: f32 = 1e-5;

    fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
        #[allow(clippy::cast_precision_loss)]
        (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
    }

    // =========================================================================
    // Correctness Tests
    // =========================================================================

    #[test]
    fn test_dot_product_simd_basic() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let result = dot_product_simd(&a, &b);
        assert!((result - 36.0).abs() < EPSILON);
    }

    #[test]
    fn test_dot_product_simd_768d() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let simd_result = dot_product_simd(&a, &b);
        let scalar_result: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        let rel_error = (simd_result - scalar_result).abs() / scalar_result.abs().max(1.0);
        assert!(rel_error < 1e-4, "Relative error too high: {rel_error}");
    }

    #[test]
    fn test_euclidean_distance_simd_identical() {
        let v = generate_test_vector(768, 0.0);
        let result = euclidean_distance_simd(&v, &v);
        assert!(
            result.abs() < EPSILON,
            "Identical vectors should have distance 0"
        );
    }

    #[test]
    fn test_euclidean_distance_simd_known() {
        let a = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let result = euclidean_distance_simd(&a, &b);
        assert!(
            (result - 5.0).abs() < EPSILON,
            "Expected 5.0 (3-4-5 triangle)"
        );
    }

    #[test]
    fn test_cosine_similarity_simd_identical() {
        let v = generate_test_vector(768, 0.0);
        let result = cosine_similarity_simd(&v, &v);
        assert!(
            (result - 1.0).abs() < EPSILON,
            "Identical vectors should have similarity 1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_simd_orthogonal() {
        let mut a = vec![0.0; 16];
        let mut b = vec![0.0; 16];
        a[0] = 1.0;
        b[1] = 1.0;
        let result = cosine_similarity_simd(&a, &b);
        assert!(
            result.abs() < EPSILON,
            "Orthogonal vectors should have similarity 0"
        );
    }

    #[test]
    fn test_cosine_similarity_simd_opposite() {
        let a = generate_test_vector(768, 0.0);
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        let result = cosine_similarity_simd(&a, &b);
        assert!(
            (result + 1.0).abs() < EPSILON,
            "Opposite vectors should have similarity -1.0"
        );
    }

    #[test]
    fn test_normalize_inplace_simd_unit() {
        let mut v = vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        normalize_inplace_simd(&mut v);

        let norm_after = norm_simd(&v);
        assert!((norm_after - 1.0).abs() < EPSILON, "Should be unit vector");
        assert!((v[0] - 0.6).abs() < EPSILON, "Expected 3/5 = 0.6");
        assert!((v[1] - 0.8).abs() < EPSILON, "Expected 4/5 = 0.8");
    }

    #[test]
    fn test_normalize_inplace_simd_zero() {
        let mut v = vec![0.0; 16];
        normalize_inplace_simd(&mut v);
        assert!(v.iter().all(|&x| x == 0.0), "Zero vector should stay zero");
    }

    // =========================================================================
    // Consistency with scalar implementation
    // =========================================================================

    #[test]
    fn test_consistency_with_scalar() {
        use crate::simd::{cosine_similarity_fast, dot_product_fast, euclidean_distance_fast};

        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let dot_scalar = dot_product_fast(&a, &b);
        let dot_simd = dot_product_simd(&a, &b);
        assert!(
            (dot_scalar - dot_simd).abs() < 1e-3,
            "Dot product mismatch: {dot_scalar} vs {dot_simd}"
        );

        let dist_scalar = euclidean_distance_fast(&a, &b);
        let dist_simd = euclidean_distance_simd(&a, &b);
        assert!(
            (dist_scalar - dist_simd).abs() < 1e-3,
            "Euclidean distance mismatch: {dist_scalar} vs {dist_simd}"
        );

        let cos_scalar = cosine_similarity_fast(&a, &b);
        let cos_simd = cosine_similarity_simd(&a, &b);
        assert!(
            (cos_scalar - cos_simd).abs() < 1e-5,
            "Cosine similarity mismatch: {cos_scalar} vs {cos_simd}"
        );
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_odd_dimensions() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // 5 elements (not multiple of 8)
        let b = vec![5.0, 4.0, 3.0, 2.0, 1.0];

        let result = dot_product_simd(&a, &b);
        let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((result - expected).abs() < EPSILON);
    }

    #[test]
    fn test_small_vectors() {
        let a = vec![3.0];
        let b = vec![4.0];
        assert!((dot_product_simd(&a, &b) - 12.0).abs() < EPSILON);
    }

    #[test]
    #[should_panic(expected = "Vector dimensions must match")]
    fn test_dimension_mismatch() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let _ = dot_product_simd(&a, &b);
    }

    // =========================================================================
    // Hamming distance tests
    // =========================================================================

    #[test]
    fn test_hamming_distance_simd_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let result = hamming_distance_simd(&a, &a);
        assert!(
            result.abs() < EPSILON,
            "Identical vectors should have distance 0"
        );
    }

    #[test]
    fn test_hamming_distance_simd_all_different() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let result = hamming_distance_simd(&a, &b);
        assert!((result - 8.0).abs() < EPSILON, "All different = 8");
    }

    #[test]
    fn test_hamming_distance_simd_partial() {
        let a = vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0];
        // Differences at positions 1, 3, 5, 7 = 4 differences
        let result = hamming_distance_simd(&a, &b);
        assert!((result - 4.0).abs() < EPSILON, "Expected 4 differences");
    }

    #[test]
    fn test_hamming_distance_simd_consistency() {
        use crate::simd::hamming_distance_fast;

        let a: Vec<f32> = (0..768)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..768)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();

        let scalar = hamming_distance_fast(&a, &b);
        let simd = hamming_distance_simd(&a, &b);

        assert!(
            (scalar - simd).abs() < 1.0,
            "Hamming mismatch: {scalar} vs {simd}"
        );
    }

    // =========================================================================
    // Binary Hamming distance tests (u64 packed)
    // =========================================================================

    #[test]
    fn test_hamming_distance_binary_identical() {
        let a = vec![0xFFFF_FFFF_FFFF_FFFFu64; 16];
        let result = hamming_distance_binary(&a, &a);
        assert_eq!(result, 0, "Identical should be 0");
    }

    #[test]
    fn test_hamming_distance_binary_all_different() {
        let a = vec![0u64; 1];
        let b = vec![0xFFFF_FFFF_FFFF_FFFFu64; 1];
        let result = hamming_distance_binary(&a, &b);
        assert_eq!(result, 64, "All 64 bits different");
    }

    #[test]
    fn test_hamming_distance_binary_known() {
        let a = vec![0b1010_1010u64];
        let b = vec![0b0101_0101u64];
        let result = hamming_distance_binary(&a, &b);
        assert_eq!(result, 8, "8 bits different in low byte");
    }

    #[test]
    fn test_hamming_distance_binary_fast_identical() {
        let a = vec![0xFFFF_FFFF_FFFF_FFFFu64; 16];
        let result = hamming_distance_binary_fast(&a, &a);
        assert_eq!(result, 0, "Identical should be 0");
    }

    #[test]
    fn test_hamming_distance_binary_fast_all_different() {
        let a = vec![0u64; 16];
        let b = vec![0xFFFF_FFFF_FFFF_FFFFu64; 16];
        let result = hamming_distance_binary_fast(&a, &b);
        assert_eq!(result, 64 * 16, "All bits different");
    }

    #[test]
    fn test_hamming_distance_binary_fast_consistency() {
        let a: Vec<u64> = (0..24).map(|i| i * 0x1234_5678).collect();
        let b: Vec<u64> = (0..24).map(|i| i * 0x8765_4321).collect();

        let standard = hamming_distance_binary(&a, &b);
        let fast = hamming_distance_binary_fast(&a, &b);

        assert_eq!(standard, fast, "Fast should match standard");
    }

    // =========================================================================
    // Jaccard similarity tests
    // =========================================================================

    #[test]
    fn test_jaccard_similarity_simd_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let result = jaccard_similarity_simd(&a, &a);
        assert!((result - 1.0).abs() < EPSILON, "Identical = 1.0");
    }

    #[test]
    fn test_jaccard_similarity_simd_disjoint() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let result = jaccard_similarity_simd(&a, &b);
        assert!(result.abs() < EPSILON, "Disjoint sets = 0.0");
    }

    #[test]
    fn test_jaccard_similarity_simd_half_overlap() {
        let a = vec![1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        // Intersection = 1 (position 0), Union = 3 (positions 0, 1, 2)
        let result = jaccard_similarity_simd(&a, &b);
        assert!((result - (1.0 / 3.0)).abs() < EPSILON, "Expected 1/3");
    }

    #[test]
    fn test_jaccard_similarity_simd_empty() {
        let a = vec![0.0; 16];
        let b = vec![0.0; 16];
        let result = jaccard_similarity_simd(&a, &b);
        assert!((result - 1.0).abs() < EPSILON, "Empty sets = 1.0");
    }

    #[test]
    fn test_jaccard_similarity_simd_consistency() {
        use crate::simd::jaccard_similarity_fast;

        let a: Vec<f32> = (0..768)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..768)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();

        let scalar = jaccard_similarity_fast(&a, &b);
        let simd = jaccard_similarity_simd(&a, &b);

        assert!(
            (scalar - simd).abs() < 1e-4,
            "Jaccard mismatch: {scalar} vs {simd}"
        );
    }

    // =========================================================================
    // Hamming distance u32 tests (Flag 7 fix)
    // =========================================================================

    #[test]
    fn test_hamming_distance_simd_u32_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let result = hamming_distance_simd_u32(&a, &a);
        assert_eq!(result, 0, "Identical vectors should have distance 0");
    }

    #[test]
    fn test_hamming_distance_simd_u32_all_different() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let result = hamming_distance_simd_u32(&a, &b);
        assert_eq!(result, 8, "All different = 8");
    }

    #[test]
    fn test_hamming_distance_simd_u32_partial() {
        let a = vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0];
        let result = hamming_distance_simd_u32(&a, &b);
        assert_eq!(result, 4, "Expected 4 differences");
    }

    #[test]
    fn test_hamming_distance_simd_u32_consistency_with_f32() {
        let a: Vec<f32> = (0..768)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..768)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();

        let f32_result = hamming_distance_simd(&a, &b);
        let u32_result = hamming_distance_simd_u32(&a, &b);

        #[allow(clippy::cast_precision_loss)]
        let expected_f32 = u32_result as f32;
        assert!(
            (f32_result - expected_f32).abs() < 1.0,
            "u32 and f32 results should match: {u32_result} vs {f32_result}"
        );
    }
}
