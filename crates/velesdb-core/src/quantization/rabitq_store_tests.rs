//! Tests for `RaBitQVectorStore`.

use crate::quantization::RaBitQCorrection;
use super::RaBitQVectorStore;

#[test]
fn test_new_store_is_empty() {
    let store = RaBitQVectorStore::new(128, 100);
    assert!(store.is_empty());
    assert_eq!(store.len(), 0);
    assert_eq!(store.dimension(), 128);
}

#[test]
fn test_push_and_get() {
    let dim: usize = 128;
    let words = dim.div_ceil(64); // 2 words
    let mut store = RaBitQVectorStore::new(dim, 10);

    let bits = vec![0xDEAD_BEEF_CAFE_BABEu64; words];
    let correction = RaBitQCorrection {
        vector_norm: 1.5,
        quantization_ip: 0.95,
    };

    store.push(&bits, correction);

    assert_eq!(store.len(), 1);
    assert!(!store.is_empty());

    let retrieved_bits = store.get_bits_slice(0).unwrap();
    assert_eq!(retrieved_bits, &bits[..]);

    let retrieved_corr = store.get_correction(0).unwrap();
    assert!((retrieved_corr.vector_norm - 1.5).abs() < f32::EPSILON);
    assert!((retrieved_corr.quantization_ip - 0.95).abs() < f32::EPSILON);
}

#[test]
fn test_out_of_bounds_returns_none() {
    let store = RaBitQVectorStore::new(64, 10);
    assert!(store.get_bits_slice(0).is_none());
    assert!(store.get_correction(0).is_none());
}

#[test]
fn test_multiple_vectors() {
    let dim: usize = 64;
    let words = dim.div_ceil(64); // 1 word
    let mut store = RaBitQVectorStore::new(dim, 10);

    let norms: [f32; 5] = [0.0, 1.0, 2.0, 3.0, 4.0];
    for (i, &norm) in norms.iter().enumerate() {
        let bits = vec![(i as u64) * 1000];
        let correction = RaBitQCorrection {
            vector_norm: norm,
            quantization_ip: 0.9,
        };
        store.push(&bits, correction);
    }

    assert_eq!(store.len(), 5);

    // Verify each vector is independent
    for (i, &norm) in norms.iter().enumerate() {
        let bits = store.get_bits_slice(i).unwrap();
        assert_eq!(bits.len(), words);
        assert_eq!(bits[0], (i as u64) * 1000);

        let corr = store.get_correction(i).unwrap();
        assert!((corr.vector_norm - norm).abs() < f32::EPSILON);
    }
}

#[test]
fn test_prefetch_does_not_panic() {
    let dim: usize = 128;
    let mut store = RaBitQVectorStore::new(dim, 10);
    let bits = vec![0u64; dim.div_ceil(64)];
    let correction = RaBitQCorrection {
        vector_norm: 1.0,
        quantization_ip: 0.9,
    };
    store.push(&bits, correction);

    // Should not panic even for valid and invalid indices
    store.prefetch(0);
    store.prefetch(999); // out of bounds — no-op
}
