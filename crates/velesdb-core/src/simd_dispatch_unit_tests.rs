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
