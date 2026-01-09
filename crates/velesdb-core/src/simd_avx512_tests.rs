//! Tests for `simd_avx512` module

#[cfg(test)]
mod tests {
    use crate::simd_avx512::*;

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

    // =========================================================================
    // Pre-normalized vector tests
    // =========================================================================

    #[test]
    fn test_cosine_similarity_normalized_identical() {
        let mut v = generate_test_vector(768, 0.0);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut v {
            *x /= norm;
        }

        let result = cosine_similarity_normalized(&v, &v);
        assert!(
            (result - 1.0).abs() < EPSILON,
            "Identical unit vectors should have similarity 1.0, got {result}"
        );
    }

    #[test]
    fn test_cosine_similarity_normalized_orthogonal() {
        let mut a = vec![0.0; 768];
        let mut b = vec![0.0; 768];
        a[0] = 1.0; // Unit vector along x
        b[1] = 1.0; // Unit vector along y

        let result = cosine_similarity_normalized(&a, &b);
        assert!(
            result.abs() < EPSILON,
            "Orthogonal unit vectors should have similarity 0, got {result}"
        );
    }

    #[test]
    fn test_cosine_similarity_normalized_matches_auto() {
        let mut a = generate_test_vector(768, 0.0);
        let mut b = generate_test_vector(768, 1.0);

        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut a {
            *x /= norm_a;
        }
        for x in &mut b {
            *x /= norm_b;
        }

        let normalized = cosine_similarity_normalized(&a, &b);
        let auto = cosine_similarity_auto(&a, &b);

        assert!(
            (normalized - auto).abs() < 1e-4,
            "Normalized and auto should match for unit vectors: {normalized} vs {auto}"
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn test_batch_cosine_normalized() {
        let mut vectors: Vec<Vec<f32>> = (0..10)
            .map(|i| generate_test_vector(768, i as f32))
            .collect();

        // Normalize all vectors
        for v in &mut vectors {
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v {
                *x /= norm;
            }
        }

        let mut query = generate_test_vector(768, 100.0);
        let norm_q: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut query {
            *x /= norm_q;
        }

        let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        let results = batch_cosine_normalized(&refs, &query);

        assert_eq!(results.len(), 10);
        for r in &results {
            assert!((-1.0..=1.0).contains(r), "Cosine must be in [-1, 1]");
        }
    }
}
