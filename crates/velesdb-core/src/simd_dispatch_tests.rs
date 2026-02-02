//! Tests for `simd_dispatch` module - Runtime SIMD dispatch.

use crate::simd_dispatch::{
    cosine_dispatched, cosine_normalized_dispatched, dot_product_dispatched, euclidean_dispatched,
    hamming_dispatched, prefetch_distance, simd_features_info, SimdFeatures,
    PREFETCH_DISTANCE_1536D, PREFETCH_DISTANCE_384D, PREFETCH_DISTANCE_768D,
};

// -------------------------------------------------------------------------
// Dispatch correctness tests
// -------------------------------------------------------------------------

#[test]
fn test_dot_product_dispatched_correctness() {
    // Arrange
    let a = vec![1.0f32, 2.0, 3.0, 4.0];
    let b = vec![5.0f32, 6.0, 7.0, 8.0];

    // Act
    let result = dot_product_dispatched(&a, &b);

    // Assert - 1*5 + 2*6 + 3*7 + 4*8 = 5 + 12 + 21 + 32 = 70
    assert!((result - 70.0).abs() < 1e-5);
}

#[test]
fn test_euclidean_dispatched_correctness() {
    // Arrange
    let a = vec![0.0f32, 0.0, 0.0];
    let b = vec![3.0f32, 4.0, 0.0];

    // Act
    let result = euclidean_dispatched(&a, &b);

    // Assert - sqrt(9 + 16) = 5
    assert!((result - 5.0).abs() < 1e-5);
}

#[test]
fn test_cosine_dispatched_correctness() {
    // Arrange - same vector should have cosine = 1.0
    let a = vec![1.0f32, 2.0, 3.0];
    let b = vec![1.0f32, 2.0, 3.0];

    // Act
    let result = cosine_dispatched(&a, &b);

    // Assert
    assert!((result - 1.0).abs() < 1e-5);
}

#[test]
fn test_cosine_dispatched_orthogonal() {
    // Arrange - orthogonal vectors should have cosine = 0
    let a = vec![1.0f32, 0.0, 0.0];
    let b = vec![0.0f32, 1.0, 0.0];

    // Act
    let result = cosine_dispatched(&a, &b);

    // Assert
    assert!(result.abs() < 1e-5);
}

#[test]
fn test_cosine_normalized_dispatched() {
    // Arrange - pre-normalized vectors
    let a = vec![1.0f32, 0.0];
    let b = vec![0.707f32, 0.707]; // ~45 degrees

    // Act
    let result = cosine_normalized_dispatched(&a, &b);

    // Assert - cos(45°) ≈ 0.707
    assert!((result - 0.707).abs() < 0.01);
}

#[test]
fn test_hamming_dispatched_correctness() {
    // Arrange - binary vectors encoded as f32
    let a = vec![1.0f32, 0.0, 1.0, 0.0]; // bits: 1010
    let b = vec![1.0f32, 1.0, 0.0, 0.0]; // bits: 1100

    // Act
    let result = hamming_dispatched(&a, &b);

    // Assert - differs in positions 1 and 2
    assert_eq!(result, 2);
}

// -------------------------------------------------------------------------
// Large vector tests (768D like real embeddings)
// -------------------------------------------------------------------------

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_dot_product_dispatched_768d() {
    // Arrange
    let a: Vec<f32> = (0..768).map(|i| (i as f32) * 0.001).collect();
    let b: Vec<f32> = (0..768).map(|i| ((768 - i) as f32) * 0.001).collect();

    // Act
    let result = dot_product_dispatched(&a, &b);

    // Assert - just verify it doesn't panic and returns reasonable value
    assert!(result.is_finite());
    assert!(result > 0.0);
}

#[test]
fn test_euclidean_dispatched_768d() {
    // Arrange
    let a: Vec<f32> = vec![0.0; 768];
    let b: Vec<f32> = vec![1.0; 768];

    // Act
    let result = euclidean_dispatched(&a, &b);

    // Assert - sqrt(768 * 1) ≈ 27.71
    assert!((result - 768.0_f32.sqrt()).abs() < 0.01);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_cosine_dispatched_768d() {
    // Arrange
    let a: Vec<f32> = (0..768).map(|i| (i as f32).sin()).collect();
    let b = a.clone();

    // Act
    let result = cosine_dispatched(&a, &b);

    // Assert - same vector = 1.0
    assert!((result - 1.0).abs() < 1e-4);
}

// -------------------------------------------------------------------------
// SIMD features detection tests
// -------------------------------------------------------------------------

#[test]
fn test_simd_features_detect() {
    // Act
    let features = SimdFeatures::detect();

    // Assert - just verify it doesn't panic
    let _name = features.best_instruction_set();
    println!("SIMD features: {:?}", features);
    println!("Best instruction set: {}", features.best_instruction_set());
}

#[test]
fn test_simd_features_info() {
    // Act
    let features = simd_features_info();

    // Assert - returns valid struct
    assert!(!features.best_instruction_set().is_empty());
}

// -------------------------------------------------------------------------
// Prefetch constant tests
// -------------------------------------------------------------------------

#[test]
fn test_prefetch_distance_768d() {
    // 768 * 4 bytes / 64 bytes = 48 cache lines
    assert_eq!(PREFETCH_DISTANCE_768D, 48);
}

#[test]
fn test_prefetch_distance_384d() {
    // 384 * 4 bytes / 64 bytes = 24 cache lines
    assert_eq!(PREFETCH_DISTANCE_384D, 24);
}

#[test]
fn test_prefetch_distance_1536d() {
    // 1536 * 4 bytes / 64 bytes = 96 cache lines
    assert_eq!(PREFETCH_DISTANCE_1536D, 96);
}

#[test]
fn test_prefetch_distance_function() {
    assert_eq!(prefetch_distance(768), 48);
    assert_eq!(prefetch_distance(384), 24);
    assert_eq!(prefetch_distance(128), 8);
}

// -------------------------------------------------------------------------
// OnceLock initialization tests
// -------------------------------------------------------------------------

#[test]
fn test_dispatch_initialized_once() {
    // Multiple calls should use cached function pointer
    let a = vec![1.0f32; 100];
    let b = vec![2.0f32; 100];

    // First call initializes
    let r1 = dot_product_dispatched(&a, &b);

    // Second call uses cached pointer
    let r2 = dot_product_dispatched(&a, &b);

    // Results should be identical
    assert!((r1 - r2).abs() < f32::EPSILON);
}

#[test]
fn test_dispatch_thread_safe() {
    use std::sync::Arc;
    use std::thread;

    // Arrange
    let a = Arc::new(vec![1.0f32; 768]);
    let b = Arc::new(vec![2.0f32; 768]);

    // Act - multiple threads calling dispatched functions
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let a = Arc::clone(&a);
            let b = Arc::clone(&b);
            thread::spawn(move || {
                for _ in 0..100 {
                    let _ = dot_product_dispatched(&a, &b);
                    let _ = cosine_dispatched(&a, &b);
                    let _ = euclidean_dispatched(&a, &b);
                }
            })
        })
        .collect();

    // Assert - no panics
    for h in handles {
        h.join().expect("Thread should not panic");
    }
}

// -------------------------------------------------------------------------
// Edge case tests
// -------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Vector dimensions must match")]
fn test_dot_product_dispatched_length_mismatch() {
    let a = vec![1.0f32, 2.0];
    let b = vec![1.0f32, 2.0, 3.0];
    let _ = dot_product_dispatched(&a, &b);
}

#[test]
fn test_empty_vectors() {
    let a: Vec<f32> = vec![];
    let b: Vec<f32> = vec![];

    // Should not panic, returns 0
    assert!((dot_product_dispatched(&a, &b) - 0.0).abs() < f32::EPSILON);
}

#[test]
fn test_single_element() {
    let a = vec![3.0f32];
    let b = vec![4.0f32];

    assert!((dot_product_dispatched(&a, &b) - 12.0).abs() < f32::EPSILON);
}
