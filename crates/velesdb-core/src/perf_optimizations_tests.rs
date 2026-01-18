//! Tests for `perf_optimizations` module - Contiguous vector storage.

use crate::perf_optimizations::{
    batch_cosine_similarities, batch_dot_products_simd, ContiguousVectors,
};

const EPSILON: f32 = 1e-5;

// =========================================================================
// ContiguousVectors Tests
// =========================================================================

#[test]
fn test_contiguous_vectors_new() {
    let cv = ContiguousVectors::new(768, 100);
    assert_eq!(cv.dimension(), 768);
    assert_eq!(cv.len(), 0);
    assert!(cv.is_empty());
    assert!(cv.capacity() >= 100);
}

#[test]
fn test_contiguous_vectors_push() {
    let mut cv = ContiguousVectors::new(3, 10);
    let v1 = vec![1.0, 2.0, 3.0];
    let v2 = vec![4.0, 5.0, 6.0];

    cv.push(&v1);
    assert_eq!(cv.len(), 1);

    cv.push(&v2);
    assert_eq!(cv.len(), 2);

    let retrieved = cv.get(0).unwrap();
    assert_eq!(retrieved, &v1[..]);

    let retrieved = cv.get(1).unwrap();
    assert_eq!(retrieved, &v2[..]);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_push_batch() {
    let mut cv = ContiguousVectors::new(128, 100);
    let vectors: Vec<Vec<f32>> = (0..50)
        .map(|i| (0..128).map(|j| (i * 128 + j) as f32).collect())
        .collect();

    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    let added = cv.push_batch(refs.into_iter());

    assert_eq!(added, 50);
    assert_eq!(cv.len(), 50);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_grow() {
    let mut cv = ContiguousVectors::new(64, 16);
    let vector: Vec<f32> = (0..64).map(|i| i as f32).collect();

    // Push more than initial capacity
    for _ in 0..50 {
        cv.push(&vector);
    }

    assert_eq!(cv.len(), 50);
    assert!(cv.capacity() >= 50);

    // Verify data integrity
    for i in 0..50 {
        let retrieved = cv.get(i).unwrap();
        assert_eq!(retrieved, &vector[..]);
    }
}

#[test]
fn test_contiguous_vectors_get_out_of_bounds() {
    let cv = ContiguousVectors::new(3, 10);
    assert!(cv.get(0).is_none());
    assert!(cv.get(100).is_none());
}

#[test]
#[should_panic(expected = "dimension mismatch")]
fn test_contiguous_vectors_dimension_mismatch() {
    let mut cv = ContiguousVectors::new(3, 10);
    cv.push(&[1.0, 2.0]); // Wrong dimension
}

#[test]
fn test_contiguous_vectors_memory_bytes() {
    let cv = ContiguousVectors::new(768, 1000);
    let expected = 1000 * 768 * 4; // capacity * dimension * sizeof(f32)
    assert!(cv.memory_bytes() >= expected);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_prefetch() {
    let mut cv = ContiguousVectors::new(64, 100);
    for i in 0..50 {
        let v: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32).collect();
        cv.push(&v);
    }

    // Should not panic
    cv.prefetch(0);
    cv.prefetch(25);
    cv.prefetch(49);
    cv.prefetch(100); // Out of bounds - should be no-op
}

#[test]
fn test_contiguous_vectors_dot_product() {
    let mut cv = ContiguousVectors::new(3, 10);
    cv.push(&[1.0, 0.0, 0.0]);
    cv.push(&[0.0, 1.0, 0.0]);

    let query = vec![1.0, 0.0, 0.0];

    let dp0 = cv.dot_product(0, &query).unwrap();
    assert!((dp0 - 1.0).abs() < EPSILON);

    let dp1 = cv.dot_product(1, &query).unwrap();
    assert!((dp1 - 0.0).abs() < EPSILON);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_batch_dot_products() {
    let mut cv = ContiguousVectors::new(64, 100);

    // Add normalized vectors
    for i in 0..50 {
        let mut v: Vec<f32> = (0..64).map(|j| ((i + j) % 10) as f32).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        cv.push(&v);
    }

    let query: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
    let indices: Vec<usize> = (0..50).collect();

    let results = cv.batch_dot_products(&indices, &query);
    assert_eq!(results.len(), 50);
}

// =========================================================================
// Batch Distance Tests
// =========================================================================

#[test]
fn test_batch_dot_products_simd() {
    let v1 = vec![1.0, 0.0, 0.0];
    let v2 = vec![0.0, 1.0, 0.0];
    let v3 = vec![0.5, 0.5, 0.0];
    let query = vec![1.0, 0.0, 0.0];

    let vectors: Vec<&[f32]> = vec![&v1, &v2, &v3];
    let results = batch_dot_products_simd(&vectors, &query);

    assert_eq!(results.len(), 3);
    assert!((results[0] - 1.0).abs() < EPSILON);
    assert!((results[1] - 0.0).abs() < EPSILON);
    assert!((results[2] - 0.5).abs() < EPSILON);
}

#[test]
fn test_batch_cosine_similarities() {
    let v1 = vec![1.0, 0.0, 0.0];
    let v2 = vec![0.0, 1.0, 0.0];
    let query = vec![1.0, 0.0, 0.0];

    let vectors: Vec<&[f32]> = vec![&v1, &v2];
    let results = batch_cosine_similarities(&vectors, &query);

    assert_eq!(results.len(), 2);
    assert!((results[0] - 1.0).abs() < EPSILON); // Same direction
    assert!((results[1] - 0.0).abs() < EPSILON); // Orthogonal
}

// =========================================================================
// Performance-Critical Tests
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_large_dimension() {
    // Test with BERT-like dimensions (768D)
    let mut cv = ContiguousVectors::new(768, 1000);

    for i in 0..100 {
        let v: Vec<f32> = (0..768).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
        cv.push(&v);
    }

    assert_eq!(cv.len(), 100);

    // Verify random access works
    let v50 = cv.get(50).unwrap();
    assert_eq!(v50.len(), 768);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_gpt4_dimension() {
    // Test with GPT-4 dimensions (1536D)
    let mut cv = ContiguousVectors::new(1536, 100);

    for i in 0..20 {
        let v: Vec<f32> = (0..1536).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
        cv.push(&v);
    }

    assert_eq!(cv.len(), 20);
    assert_eq!(cv.dimension(), 1536);
}

// =========================================================================
// Safety: get_unchecked bounds check tests (TDD)
// =========================================================================

#[test]
fn test_get_unchecked_valid_index() {
    // Arrange
    let mut cv = ContiguousVectors::new(3, 10);
    cv.push(&[1.0, 2.0, 3.0]);
    cv.push(&[4.0, 5.0, 6.0]);

    // Act - Valid indices should work
    let v0 = unsafe { cv.get_unchecked(0) };
    let v1 = unsafe { cv.get_unchecked(1) };

    // Assert
    assert_eq!(v0, &[1.0, 2.0, 3.0]);
    assert_eq!(v1, &[4.0, 5.0, 6.0]);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "index out of bounds")]
fn test_get_unchecked_panics_on_invalid_index_in_debug() {
    // Arrange
    let mut cv = ContiguousVectors::new(3, 10);
    cv.push(&[1.0, 2.0, 3.0]);

    // Act - Out of bounds index should panic in debug mode
    let _ = unsafe { cv.get_unchecked(5) };
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "index out of bounds")]
fn test_get_unchecked_panics_on_boundary_index_in_debug() {
    // Arrange
    let mut cv = ContiguousVectors::new(3, 10);
    cv.push(&[1.0, 2.0, 3.0]);
    cv.push(&[4.0, 5.0, 6.0]);

    // Act - Index == count should panic (off by one)
    let _ = unsafe { cv.get_unchecked(2) };
}

// =========================================================================
// P2 Audit: Resize panic-safety tests
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_resize_preserves_data_integrity() {
    // Arrange
    let mut cv = ContiguousVectors::new(64, 16);
    let vectors: Vec<Vec<f32>> = (0..10)
        .map(|i| (0..64).map(|j| (i * 64 + j) as f32).collect())
        .collect();

    for v in &vectors {
        cv.push(v);
    }

    // Act - Force resize by adding more vectors
    for i in 10..100 {
        let v: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32).collect();
        cv.push(&v);
    }

    // Assert - Original vectors should be intact
    for (i, expected) in vectors.iter().enumerate() {
        let actual = cv.get(i).expect("Vector should exist");
        assert_eq!(
            actual,
            expected.as_slice(),
            "Vector {i} corrupted after resize"
        );
    }
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_resize_multiple_times() {
    // Arrange - Start with minimal capacity
    let mut cv = ContiguousVectors::new(128, 16);

    // Act - Trigger multiple resizes
    for i in 0..500 {
        let v: Vec<f32> = (0..128).map(|j| (i * 128 + j) as f32).collect();
        cv.push(&v);
    }

    // Assert
    assert_eq!(cv.len(), 500);
    assert!(cv.capacity() >= 500);

    // Verify first and last vectors
    let first = cv.get(0).unwrap();
    assert!((first[0] - 0.0).abs() < f32::EPSILON);

    let last = cv.get(499).unwrap();
    #[allow(clippy::cast_precision_loss)]
    let expected = (499 * 128) as f32;
    assert!((last[0] - expected).abs() < f32::EPSILON);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_drop_after_resize_no_leak() {
    // Arrange - Create and resize multiple times
    for _ in 0..10 {
        let mut cv = ContiguousVectors::new(256, 8);

        // Trigger multiple resizes
        for i in 0..100 {
            let v: Vec<f32> = (0..256).map(|j| (i + j) as f32).collect();
            cv.push(&v);
        }

        // cv is dropped here - should not leak memory
    }

    // If we get here without memory issues, the test passes
    // Note: In a real scenario, use tools like valgrind or miri to verify
}

#[test]
fn test_ensure_capacity_idempotent() {
    // Arrange
    let mut cv = ContiguousVectors::new(64, 100);
    cv.push(&vec![1.0; 64]);

    let initial_capacity = cv.capacity();

    // Act - Call ensure_capacity multiple times with same value
    cv.ensure_capacity(50);
    cv.ensure_capacity(50);
    cv.ensure_capacity(50);

    // Assert - Capacity should not change
    assert_eq!(cv.capacity(), initial_capacity);
    assert_eq!(cv.len(), 1);
}
