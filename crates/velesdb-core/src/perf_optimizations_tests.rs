//! Tests for `perf_optimizations` module - Contiguous vector storage.

use crate::perf_optimizations::{
    batch_cosine_similarities, batch_dot_products_simd, pad_to_simd_width, ContiguousVectors,
};

const EPSILON: f32 = 1e-5;

// =========================================================================
// ContiguousVectors Tests
// =========================================================================

#[test]
fn test_contiguous_vectors_new() {
    let cv = ContiguousVectors::new(768, 100).expect("test");
    assert_eq!(cv.dimension(), 768);
    assert_eq!(cv.len(), 0);
    assert!(cv.is_empty());
    assert!(cv.capacity() >= 100);
}

#[test]
fn test_contiguous_vectors_push() {
    let mut cv = ContiguousVectors::new(3, 10).expect("test");
    let v1 = vec![1.0, 2.0, 3.0];
    let v2 = vec![4.0, 5.0, 6.0];

    cv.push(&v1).expect("test");
    assert_eq!(cv.len(), 1);

    cv.push(&v2).expect("test");
    assert_eq!(cv.len(), 2);

    let retrieved = cv.get(0).unwrap();
    assert_eq!(retrieved, &v1[..]);

    let retrieved = cv.get(1).unwrap();
    assert_eq!(retrieved, &v2[..]);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_push_batch() {
    let mut cv = ContiguousVectors::new(128, 100).expect("test");
    let vectors: Vec<Vec<f32>> = (0..50)
        .map(|i| (0..128).map(|j| (i * 128 + j) as f32).collect())
        .collect();

    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    let added = cv.push_batch(&refs).expect("test");

    assert_eq!(added, 50);
    assert_eq!(cv.len(), 50);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_grow() {
    let mut cv = ContiguousVectors::new(64, 16).expect("test");
    let vector: Vec<f32> = (0..64).map(|i| i as f32).collect();

    // Push more than initial capacity
    for _ in 0..50 {
        cv.push(&vector).expect("test");
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
    let cv = ContiguousVectors::new(3, 10).expect("test");
    assert!(cv.get(0).is_none());
    assert!(cv.get(100).is_none());
}

#[test]
fn test_contiguous_vectors_dimension_mismatch_returns_error() {
    let mut cv = ContiguousVectors::new(3, 10).expect("test");
    let result = cv.push(&[1.0, 2.0]); // Wrong dimension
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), "VELES-004");
}

#[test]
fn test_contiguous_vectors_memory_bytes() {
    let cv = ContiguousVectors::new(768, 1000).expect("test");
    let expected = 1000 * 768 * 4; // capacity * dimension * sizeof(f32)
    assert!(cv.memory_bytes() >= expected);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_prefetch() {
    let mut cv = ContiguousVectors::new(64, 100).expect("test");
    for i in 0..50 {
        let v: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32).collect();
        cv.push(&v).expect("test");
    }

    let before_len = cv.len();
    let v25_before = cv.get(25).expect("test").to_vec();
    cv.prefetch(0);
    cv.prefetch(25);
    cv.prefetch(49); // last valid index (count-1)
    cv.prefetch(50); // first OOB index (== count) — exercises the off-by-one boundary of `index < self.count`
    cv.prefetch(100); // far OOB — no-op
    cv.prefetch(usize::MAX); // extreme OOB — must not overflow offset or panic
                             // prefetch must be observably side-effect-free
    assert_eq!(cv.len(), before_len);
    assert_eq!(cv.get(25).expect("test"), v25_before.as_slice());
}

#[test]
fn test_contiguous_vectors_dot_product() {
    let mut cv = ContiguousVectors::new(3, 10).expect("test");
    cv.push(&[1.0, 0.0, 0.0]).expect("test");
    cv.push(&[0.0, 1.0, 0.0]).expect("test");

    let query = vec![1.0, 0.0, 0.0];

    let dp0 = cv.dot_product(0, &query).unwrap();
    assert!((dp0 - 1.0).abs() < EPSILON);

    let dp1 = cv.dot_product(1, &query).unwrap();
    assert!((dp1 - 0.0).abs() < EPSILON);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_vectors_batch_dot_products() {
    let mut cv = ContiguousVectors::new(64, 100).expect("test");

    // Add normalized vectors
    for i in 0..50 {
        let mut v: Vec<f32> = (0..64).map(|j| ((i + j) % 10) as f32).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        cv.push(&v).expect("test");
    }

    let query: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
    let indices: Vec<usize> = (0..50).collect();

    let results = cv.batch_dot_products(&indices, &query);
    assert_eq!(results.len(), 50);

    // Every batch result must be a real computed score, not 0/NaN/garbage.
    assert!(
        results.iter().all(|r| r.is_finite() && *r > 0.0),
        "all batch dot products must be finite and positive: {results:?}"
    );
    // Spot-check index 0 against the independently derived value.
    // v0[j] = ((0+j)%10) normalized; q[j] = j/64.0  ->  dot = 3.3243657
    assert!(
        (results[0] - 3.324_365_7).abs() < 1e-4,
        "index-0 dot product mismatch: got {}",
        results[0]
    );
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
// SIMD Padding Tests
// =========================================================================

#[test]
fn test_pad_to_simd_width_empty() {
    let padded = pad_to_simd_width(&[]);
    assert!(padded.is_empty());
}

#[test]
fn test_pad_to_simd_width_already_aligned() {
    let v: Vec<f32> = (0..8_u8).map(f32::from).collect();
    let padded = pad_to_simd_width(&v);
    assert_eq!(padded.len(), 8);
    assert_eq!(&padded[..], &v[..]);
}

#[test]
fn test_pad_to_simd_width_needs_padding() {
    let v = vec![1.0_f32, 2.0, 3.0];
    let padded = pad_to_simd_width(&v);
    assert_eq!(padded.len(), 8);
    assert_eq!(&padded[..3], &[1.0, 2.0, 3.0]);
    assert_eq!(&padded[3..], &[0.0; 5]);
}

#[test]
fn test_pad_to_simd_width_rounds_up_to_next_multiple() {
    let v = vec![1.0_f32; 9];
    let padded = pad_to_simd_width(&v);
    assert_eq!(padded.len(), 16);
    assert_eq!(&padded[..9], &[1.0; 9]);
    assert_eq!(&padded[9..], &[0.0; 7]);
}

#[test]
fn test_pad_to_simd_width_exact_multiple_16() {
    let v = vec![0.5_f32; 16];
    let padded = pad_to_simd_width(&v);
    assert_eq!(padded.len(), 16);
    assert_eq!(&padded[..], &v[..]);
}

// =========================================================================
// Performance-Critical Tests
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_large_dimension() {
    // Test with BERT-like dimensions (768D)
    let mut cv = ContiguousVectors::new(768, 1000).expect("test");

    for i in 0..100 {
        let v: Vec<f32> = (0..768).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
        cv.push(&v).expect("test");
    }

    assert_eq!(cv.len(), 100);

    // Verify random access works
    let v50 = cv.get(50).unwrap();
    assert_eq!(v50.len(), 768);
    // Spot-check actual stored values: v50[j] == ((50 + j) % 100) / 100.0
    assert!((v50[0] - 0.50).abs() < EPSILON, "v50[0]"); // (50  % 100)/100
    assert!((v50[50] - 0.00).abs() < EPSILON, "v50[50]"); // (100 % 100)/100
    assert!((v50[100] - 0.50).abs() < EPSILON, "v50[100]"); // (150 % 100)/100
    assert!((v50[767] - 0.17).abs() < EPSILON, "v50[767]"); // (817 % 100)/100
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_contiguous_gpt4_dimension() {
    // Test with GPT-4 dimensions (1536D)
    let mut cv = ContiguousVectors::new(1536, 100).expect("test");

    for i in 0..20 {
        let v: Vec<f32> = (0..1536).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
        cv.push(&v).expect("test");
    }

    assert_eq!(cv.len(), 20);
    assert_eq!(cv.dimension(), 1536);

    // Read back the first and last stored vectors and verify content,
    // so a bad copy offset / stride at GPT-4 dimension cannot slip through.
    let v0 = cv.get(0).expect("vector 0 exists");
    assert_eq!(v0.len(), 1536);
    assert!((v0[0] - 0.0).abs() < 1e-6); // i=0, j=0  -> (0 % 100) / 100.0
    assert!((v0[1] - 0.01).abs() < 1e-6); // i=0, j=1 -> (1 % 100) / 100.0

    let v19 = cv.get(19).expect("vector 19 exists");
    assert_eq!(v19.len(), 1536);
    assert!((v19[0] - 0.19).abs() < 1e-6); // i=19, j=0 -> (19 % 100) / 100.0
}

// =========================================================================
// Safety: get_unchecked bounds check tests (TDD)
// =========================================================================

#[test]
fn test_get_unchecked_valid_index() {
    // Arrange
    let mut cv = ContiguousVectors::new(3, 10).expect("test");
    cv.push(&[1.0, 2.0, 3.0]).expect("test");
    cv.push(&[4.0, 5.0, 6.0]).expect("test");

    // Act - Valid indices should work
    // SAFETY: `get_unchecked` requires index < count.
    // - Condition 1: Two vectors were pushed above, so indices 0 and 1 are valid.
    // Reason: Verify that `get_unchecked` returns correct data for in-bounds access.
    let v0 = unsafe { cv.get_unchecked(0) };
    // SAFETY: index 1 is valid — two vectors were pushed above.
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
    let mut cv = ContiguousVectors::new(3, 10).expect("test");
    cv.push(&[1.0, 2.0, 3.0]).expect("test");

    // Act - Out of bounds index should panic in debug mode
    // SAFETY: Intentionally calling `get_unchecked` with an invalid index.
    // - Condition 1: Index 5 exceeds count (1), triggering the debug_assert inside `get_unchecked`.
    // Reason: Verify that the debug bounds check panics on out-of-bounds access.
    let _ = unsafe { cv.get_unchecked(5) };
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "index out of bounds")]
fn test_get_unchecked_panics_on_boundary_index_in_debug() {
    // Arrange
    let mut cv = ContiguousVectors::new(3, 10).expect("test");
    cv.push(&[1.0, 2.0, 3.0]).expect("test");
    cv.push(&[4.0, 5.0, 6.0]).expect("test");

    // Act - Index == count should panic (off by one)
    // SAFETY: Intentionally calling `get_unchecked` with index == count.
    // - Condition 1: Index 2 equals count (2), triggering the debug_assert inside `get_unchecked`.
    // Reason: Verify that the debug bounds check catches the off-by-one boundary.
    let _ = unsafe { cv.get_unchecked(2) };
}

// =========================================================================
// P2 Audit: Resize panic-safety tests
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_resize_preserves_data_integrity() {
    // Arrange
    let mut cv = ContiguousVectors::new(64, 16).expect("test");
    let vectors: Vec<Vec<f32>> = (0..10)
        .map(|i| (0..64).map(|j| (i * 64 + j) as f32).collect())
        .collect();

    for v in &vectors {
        cv.push(v).expect("test");
    }

    // Act - Force resize by adding more vectors
    for i in 10..100 {
        let v: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32).collect();
        cv.push(&v).expect("test");
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
    let mut cv = ContiguousVectors::new(128, 16).expect("test");

    // Act - Trigger multiple resizes
    for i in 0..500 {
        let v: Vec<f32> = (0..128).map(|j| (i * 128 + j) as f32).collect();
        cv.push(&v).expect("test");
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
        let mut cv = ContiguousVectors::new(256, 8).expect("test");

        // Trigger multiple resizes
        for i in 0..100 {
            let v: Vec<f32> = (0..256).map(|j| (i + j) as f32).collect();
            cv.push(&v).expect("test");
        }
        // resize path must preserve count, grow capacity, and keep data intact
        assert_eq!(cv.len(), 100);
        assert!(cv.capacity() >= 100); // grew from 8 via doubling; >= avoids over-fitting
        let last = cv.get(99).expect("last vector present after resizes");
        assert!((last[0] - 99.0).abs() < EPSILON); // value survived buffer migration
                                                   // cv is dropped here; under `cargo miri test --lib` (scripts/local-ci.ps1 -Miri)
                                                   // a leak or layout-mismatched dealloc in resize()/Drop fails the test.
    }
}

#[test]
fn test_ensure_capacity_idempotent() {
    // Arrange
    let mut cv = ContiguousVectors::new(64, 100).expect("test");
    cv.push(&vec![1.0; 64]).expect("test");

    let initial_capacity = cv.capacity();

    // Act - Call ensure_capacity multiple times with same value
    cv.ensure_capacity(50).expect("test");
    cv.ensure_capacity(50).expect("test");
    cv.ensure_capacity(50).expect("test");

    // Assert - Capacity should not change
    assert_eq!(cv.capacity(), initial_capacity);
    assert_eq!(cv.len(), 1);
}

// =========================================================================
// P2 Audit: Error handling tests (no panics in production)
// =========================================================================

#[test]
fn test_new_zero_dimension_returns_error() {
    let result = ContiguousVectors::new(0, 100);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), "VELES-032");
}

#[test]
fn test_new_overflow_dimension_returns_error() {
    // Requesting absurd sizes must return an error, not panic. Since #899
    // enforces the dimension range up front, an absurd dimension is rejected as
    // InvalidDimension (VELES-032) before the size product is ever computed.
    let result = ContiguousVectors::new(usize::MAX / 2, usize::MAX / 2);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), "VELES-032");
}

#[test]
fn test_new_overflow_capacity_returns_alloc_error() {
    // With a valid dimension but an absurd capacity, the size product overflows /
    // exceeds the ceiling and surfaces as AllocationFailed (VELES-033), not a panic.
    let result = ContiguousVectors::new(crate::validation::MAX_DIMENSION, usize::MAX / 2);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), "VELES-033");
}

// =========================================================================
// Phase 2: GPU zero-copy access methods
// =========================================================================

#[test]
fn test_contiguous_vectors_as_flat_slice() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    cv.push(&[1.0, 2.0, 3.0, 4.0]).expect("test");
    cv.push(&[5.0, 6.0, 7.0, 8.0]).expect("test");
    cv.push(&[9.0, 10.0, 11.0, 12.0]).expect("test");

    let flat = cv.as_flat_slice();
    assert_eq!(flat.len(), 12);
    assert_eq!(
        flat,
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0]
    );
}

#[test]
fn test_contiguous_vectors_as_flat_slice_empty() {
    let cv = ContiguousVectors::new(4, 16).expect("test");
    let flat = cv.as_flat_slice();
    assert!(flat.is_empty());
}

#[test]
fn test_contiguous_vectors_gather_flat() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    cv.push(&[1.0, 2.0, 3.0, 4.0]).expect("test");
    cv.push(&[5.0, 6.0, 7.0, 8.0]).expect("test");
    cv.push(&[9.0, 10.0, 11.0, 12.0]).expect("test");
    cv.push(&[13.0, 14.0, 15.0, 16.0]).expect("test");

    // Gather indices 0 and 2
    let gathered = cv.gather_flat(&[0, 2]);
    assert_eq!(gathered.len(), 8);
    assert_eq!(gathered, vec![1.0, 2.0, 3.0, 4.0, 9.0, 10.0, 11.0, 12.0]);
}

#[test]
fn test_contiguous_vectors_gather_flat_empty_indices() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    cv.push(&[1.0, 2.0, 3.0, 4.0]).expect("test");

    let gathered = cv.gather_flat(&[]);
    assert!(gathered.is_empty());
}

#[test]
fn test_contiguous_vectors_gather_flat_all() {
    let mut cv = ContiguousVectors::new(3, 16).expect("test");
    cv.push(&[1.0, 2.0, 3.0]).expect("test");
    cv.push(&[4.0, 5.0, 6.0]).expect("test");

    let gathered = cv.gather_flat(&[0, 1]);
    assert_eq!(gathered, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

// =========================================================================
// I2: Pre-Allocated Vector Storage for Batch Insert
// =========================================================================

#[test]
fn test_reserve_additional_grows_capacity() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    cv.push(&[1.0, 2.0, 3.0, 4.0]).expect("test");

    cv.reserve_additional(1000)
        .expect("test: reserve should succeed");

    // reserve_additional guarantees capacity >= len + additional
    assert!(
        cv.capacity() >= cv.len() + 1000,
        "capacity should be >= len + 1000: len={}, capacity={}",
        cv.len(),
        cv.capacity()
    );
    // Data must remain intact after capacity growth
    assert_eq!(cv.len(), 1);
    assert_eq!(cv.get(0).expect("test"), &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn test_reserve_additional_noop_when_sufficient() {
    let mut cv = ContiguousVectors::new(4, 1000).expect("test");
    cv.push(&[1.0, 2.0, 3.0, 4.0]).expect("test");

    let cap_before = cv.capacity();
    // Already has 999 slots free, asking for 100 more should be a no-op
    cv.reserve_additional(100)
        .expect("test: reserve should succeed");

    assert_eq!(cv.capacity(), cap_before, "capacity should not change");
    assert_eq!(cv.len(), 1);
}

#[test]
fn test_reserve_additional_zero_is_noop() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    let cap_before = cv.capacity();

    cv.reserve_additional(0)
        .expect("test: reserve zero should succeed");
    assert_eq!(cv.capacity(), cap_before);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_reserve_additional_then_push_batch_no_resize() {
    let mut cv = ContiguousVectors::new(32, 16).expect("test");

    // Pre-reserve space for 500 vectors
    cv.reserve_additional(500)
        .expect("test: reserve should succeed");
    let cap_after_reserve = cv.capacity();

    // Push 500 vectors — should NOT trigger any resize
    let vectors: Vec<Vec<f32>> = (0..500)
        .map(|i| (0..32).map(|j| (i * 32 + j) as f32 * 0.001).collect())
        .collect();
    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    let added = cv
        .push_batch(&refs)
        .expect("test: push_batch should succeed");

    assert_eq!(added, 500);
    assert_eq!(cv.len(), 500);
    // Capacity should be unchanged — no resize was triggered
    assert_eq!(
        cv.capacity(),
        cap_after_reserve,
        "push_batch should not trigger resize after reserve_additional"
    );

    // Verify data integrity
    for (i, expected) in vectors.iter().enumerate() {
        let actual = cv.get(i).expect("test: vector should exist");
        assert_eq!(actual, expected.as_slice(), "vector {i} data mismatch");
    }
}

// =========================================================================
// #899 — Checked arithmetic / allocation-bound regression tests
// =========================================================================

/// `insert_at(usize::MAX, ..)` must error (no `index + 1` wrap to 0 → OOB write).
#[test]
fn test_insert_at_usize_max_index_rejected() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    let v = vec![1.0_f32, 2.0, 3.0, 4.0];
    let err = cv
        .insert_at(usize::MAX, &v)
        .expect_err("usize::MAX index must be rejected, not wrap to OOB write");
    assert!(matches!(err, crate::error::Error::AllocationFailed(_)));
    // State must be unchanged — no partial/OOB write occurred.
    assert_eq!(cv.len(), 0);
}

/// A large-but-not-MAX index whose `index * dimension` offset overflows must error.
#[test]
fn test_insert_at_offset_overflow_rejected() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    let v = vec![1.0_f32, 2.0, 3.0, 4.0];
    // index * 4 overflows usize for this index.
    let index = usize::MAX / 2;
    let err = cv
        .insert_at(index, &v)
        .expect_err("offset overflow must be rejected");
    assert!(matches!(err, crate::error::Error::AllocationFailed(_)));
    assert_eq!(cv.len(), 0);
}

/// Oversized dimension (> MAX_DIMENSION) must be rejected at construction.
#[test]
fn test_oversized_dimension_rejected_at_construction() {
    let err = ContiguousVectors::new(crate::validation::MAX_DIMENSION + 1, 16)
        .expect_err("dimension above MAX_DIMENSION must be rejected");
    assert!(matches!(err, crate::error::Error::InvalidDimension { .. }));
    // The maximum valid dimension must still succeed.
    assert!(ContiguousVectors::new(crate::validation::MAX_DIMENSION, 1).is_ok());
}

/// A construction request above the AllocGuard byte ceiling must be rejected.
///
/// #899 follow-up: the default ceiling is now a high 1 TiB backstop, so this
/// uses a deliberately impossible ~238 TiB request (still below `usize`
/// overflow) to exercise the ceiling rejection itself rather than the checked
/// arithmetic. A legitimate large index (tens/hundreds of GiB) is NOT rejected.
#[test]
fn test_construction_above_alloc_ceiling_rejected() {
    // dimension * capacity * 4 well above the 1 TiB default ceiling, but each
    // factor is small enough not to overflow usize on its own.
    let dimension = 65_536; // == MAX_DIMENSION, a valid dimension
    let capacity = 1_000_000_000; // 65_536 * 1e9 * 4 bytes ≈ 238 TiB ≫ 1 TiB
    let err = ContiguousVectors::new(dimension, capacity)
        .expect_err("allocation above ceiling must be rejected");
    assert!(matches!(err, crate::error::Error::AllocationFailed(_)));
}

/// REGRESSION (#899 follow-up): a large-but-legitimate single buffer that the
/// old 16 GiB cap would have falsely rejected now constructs successfully —
/// only the metadata/decision is exercised; we do NOT actually back 20 GiB.
///
/// `byte_size` is `pub(crate)`, so this test (compiled into the crate) can call
/// the exact bound-decision path used by `new` without allocating.
#[test]
fn test_large_legit_buffer_size_decision_accepted() {
    const GIB: usize = 1024 * 1024 * 1024;
    // 20 GiB at 768D ≈ 6.8M vectors — above the old 16 GiB cap, below 1 TiB.
    let dimension = 768;
    let capacity = (20 * GIB) / (dimension * std::mem::size_of::<f32>());
    let bytes = ContiguousVectors::byte_size(dimension, capacity)
        .expect("byte_size must not overflow for a legit large buffer");
    assert!(
        bytes > 16 * GIB,
        "test setup: should exceed the old 16 GiB cap"
    );
    assert!(
        crate::alloc_guard::check_alloc_bound(bytes).is_ok(),
        "legit ~20 GiB single buffer must not be falsely rejected"
    );
}

/// Geometric-growth doubling must not wrap: a resize request near usize::MAX
/// fails cleanly rather than wrapping `capacity * 2` to a tiny value.
#[test]
fn test_ensure_capacity_doubling_overflow_handled() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    let err = cv
        .ensure_capacity(usize::MAX)
        .expect_err("usize::MAX capacity must fail cleanly");
    assert!(matches!(err, crate::error::Error::AllocationFailed(_)));
    // Buffer remains usable after the rejected growth.
    assert_eq!(cv.capacity(), 16);
    cv.push(&[1.0, 2.0, 3.0, 4.0]).expect("test");
    assert_eq!(cv.len(), 1);
}

/// `gather_flat` with a huge index list must not panic on the capacity hint and
/// must skip out-of-bounds indices.
#[test]
fn test_gather_flat_overflow_hint_safe() {
    let mut cv = ContiguousVectors::new(4, 16).expect("test");
    cv.push(&[1.0, 2.0, 3.0, 4.0]).expect("test");
    // Valid index plus an out-of-bounds one; result holds only the valid vector.
    let out = cv.gather_flat(&[0, 999]);
    assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
}

/// Normal store / insert / gather paths remain unaffected by the new guards.
#[test]
fn test_normal_paths_unaffected_by_guards() {
    let mut cv = ContiguousVectors::new(3, 16).expect("test");
    cv.insert_at(0, &[1.0, 2.0, 3.0]).expect("test");
    cv.insert_at(2, &[7.0, 8.0, 9.0]).expect("test"); // sparse insert
    cv.push(&[4.0, 5.0, 6.0]).expect("test");
    assert_eq!(cv.get(0), Some(&[1.0, 2.0, 3.0][..]));
    assert_eq!(cv.get(2), Some(&[7.0, 8.0, 9.0][..]));
    let flat = cv.gather_flat(&[0, 2]);
    assert_eq!(flat, vec![1.0, 2.0, 3.0, 7.0, 8.0, 9.0]);
}
