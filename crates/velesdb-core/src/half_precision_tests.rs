//! Tests for `half_precision` module

use super::half_precision::*;

const EPSILON: f32 = 1e-3; // Relaxed for f16 precision loss

fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

// =========================================================================
// VectorPrecision tests
// =========================================================================

#[test]
fn test_precision_bytes_per_element() {
    assert_eq!(VectorPrecision::F32.bytes_per_element(), 4);
    assert_eq!(VectorPrecision::F16.bytes_per_element(), 2);
    assert_eq!(VectorPrecision::BF16.bytes_per_element(), 2);
}

#[test]
fn test_precision_memory_size() {
    // 768D BERT embedding
    assert_eq!(VectorPrecision::F32.memory_size(768), 3072); // 3 KB
    assert_eq!(VectorPrecision::F16.memory_size(768), 1536); // 1.5 KB
    assert_eq!(VectorPrecision::BF16.memory_size(768), 1536); // 1.5 KB
}

#[test]
fn test_precision_default() {
    assert_eq!(VectorPrecision::default(), VectorPrecision::F32);
}

// =========================================================================
// VectorData creation tests
// =========================================================================

#[test]
fn test_vector_data_from_f32_slice_f32() {
    let data = vec![0.1, 0.2, 0.3];
    let v = VectorData::from_f32_slice(&data, VectorPrecision::F32);

    assert_eq!(v.precision(), VectorPrecision::F32);
    assert_eq!(v.len(), 3);
    assert!(!v.is_empty());
}

#[test]
fn test_vector_data_from_f32_slice_f16() {
    let data = vec![0.1, 0.2, 0.3];
    let v = VectorData::from_f32_slice(&data, VectorPrecision::F16);

    assert_eq!(v.precision(), VectorPrecision::F16);
    assert_eq!(v.len(), 3);
}

#[test]
fn test_vector_data_from_f32_slice_bf16() {
    let data = vec![0.1, 0.2, 0.3];
    let v = VectorData::from_f32_slice(&data, VectorPrecision::BF16);

    assert_eq!(v.precision(), VectorPrecision::BF16);
    assert_eq!(v.len(), 3);
}

#[test]
#[allow(clippy::similar_names)]
fn test_vector_data_memory_size() {
    let data = generate_test_vector(768, 0.0);

    let full = VectorData::from_f32_slice(&data, VectorPrecision::F32);
    let half = VectorData::from_f32_slice(&data, VectorPrecision::F16);
    let brain = VectorData::from_f32_slice(&data, VectorPrecision::BF16);

    assert_eq!(full.memory_size(), 3072);
    assert_eq!(half.memory_size(), 1536);
    assert_eq!(brain.memory_size(), 1536);

    // 50% memory reduction
    assert_eq!(half.memory_size(), full.memory_size() / 2);
}

// =========================================================================
// Conversion tests
// =========================================================================

#[test]
fn test_vector_data_to_f32_roundtrip() {
    let original = vec![0.1, 0.5, 1.0, -0.5, 0.0];

    // F32 -> F32 (exact)
    let v_f32 = VectorData::from_f32_slice(&original, VectorPrecision::F32);
    let back = v_f32.to_f32_vec();
    assert_eq!(original, back);
}

#[test]
fn test_vector_data_f16_roundtrip_precision() {
    let original = vec![0.1, 0.5, 1.0, -0.5, 0.0];

    let v_f16 = VectorData::from_f32_slice(&original, VectorPrecision::F16);
    let back = v_f16.to_f32_vec();

    // f16 has ~3.3 decimal digits of precision
    for (orig, converted) in original.iter().zip(back.iter()) {
        assert!(
            (orig - converted).abs() < 0.001,
            "f16 roundtrip error: {orig} vs {converted}"
        );
    }
}

#[test]
fn test_vector_data_bf16_roundtrip_precision() {
    let original = vec![0.1, 0.5, 1.0, -0.5, 0.0];

    let v_bf16 = VectorData::from_f32_slice(&original, VectorPrecision::BF16);
    let back = v_bf16.to_f32_vec();

    // bf16 has ~2.4 decimal digits of precision
    for (orig, converted) in original.iter().zip(back.iter()) {
        assert!(
            (orig - converted).abs() < 0.01,
            "bf16 roundtrip error: {orig} vs {converted}"
        );
    }
}

#[test]
fn test_vector_data_convert() {
    let data = vec![0.1, 0.2, 0.3];
    let original = VectorData::from_f32_slice(&data, VectorPrecision::F32);

    let to_half = original.convert(VectorPrecision::F16);
    assert_eq!(to_half.precision(), VectorPrecision::F16);

    let to_brain = original.convert(VectorPrecision::BF16);
    assert_eq!(to_brain.precision(), VectorPrecision::BF16);

    // Same precision returns clone
    let same = original.convert(VectorPrecision::F32);
    assert_eq!(same.precision(), VectorPrecision::F32);
}

#[test]
fn test_vector_data_as_f32_slice() {
    let data = vec![0.1, 0.2, 0.3];

    let v_f32 = VectorData::from_f32_slice(&data, VectorPrecision::F32);
    assert!(v_f32.as_f32_slice().is_some());

    let v_f16 = VectorData::from_f32_slice(&data, VectorPrecision::F16);
    assert!(v_f16.as_f32_slice().is_none());
}

// =========================================================================
// Distance calculation tests
// =========================================================================

#[test]
fn test_dot_product_f32() {
    let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
    let b = VectorData::from_f32_slice(&[4.0, 5.0, 6.0], VectorPrecision::F32);

    let result = dot_product(&a, &b);
    assert!(
        (result - 32.0).abs() < EPSILON,
        "Expected 32.0, got {result}"
    );
}

#[test]
fn test_dot_product_f16() {
    let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F16);
    let b = VectorData::from_f32_slice(&[4.0, 5.0, 6.0], VectorPrecision::F16);

    let result = dot_product(&a, &b);
    assert!((result - 32.0).abs() < 0.1, "f16 dot product: got {result}");
}

#[test]
fn test_dot_product_bf16() {
    let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::BF16);
    let b = VectorData::from_f32_slice(&[4.0, 5.0, 6.0], VectorPrecision::BF16);

    let result = dot_product(&a, &b);
    assert!(
        (result - 32.0).abs() < 0.5,
        "bf16 dot product: got {result}"
    );
}

#[test]
fn test_cosine_similarity_identical_f16() {
    let data = generate_test_vector(768, 0.0);
    let a = VectorData::from_f32_slice(&data, VectorPrecision::F16);
    let b = VectorData::from_f32_slice(&data, VectorPrecision::F16);

    let result = cosine_similarity(&a, &b);
    assert!(
        (result - 1.0).abs() < 0.01,
        "Identical f16 vectors cosine ≈ 1.0, got {result}"
    );
}

#[test]
fn test_euclidean_distance_f16() {
    let a = VectorData::from_f32_slice(&[0.0, 0.0, 0.0], VectorPrecision::F16);
    let b = VectorData::from_f32_slice(&[3.0, 4.0, 0.0], VectorPrecision::F16);

    let result = euclidean_distance(&a, &b);
    assert!(
        (result - 5.0).abs() < 0.1,
        "f16 euclidean 3-4-5: got {result}"
    );
}

#[test]
fn test_mixed_precision_distance() {
    let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
    let b = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F16);

    // Should work with mixed precision
    let result = cosine_similarity(&a, &b);
    assert!(
        (result - 1.0).abs() < 0.01,
        "Mixed precision cosine: got {result}"
    );
}

// =========================================================================
// Precision impact tests (recall quality)
// =========================================================================

#[test]
fn test_f16_preserves_ranking() {
    // Verify that f16 preserves relative ordering of distances
    let query = generate_test_vector(768, 0.0);
    let close = generate_test_vector(768, 0.1); // Similar
    let far = generate_test_vector(768, 5.0); // Different

    // F32 distances
    let q_f32 = VectorData::from_f32_slice(&query, VectorPrecision::F32);
    let close_f32 = VectorData::from_f32_slice(&close, VectorPrecision::F32);
    let far_f32 = VectorData::from_f32_slice(&far, VectorPrecision::F32);

    let dist_close_f32 = cosine_similarity(&q_f32, &close_f32);
    let dist_far_f32 = cosine_similarity(&q_f32, &far_f32);

    // F16 distances
    let q_f16 = VectorData::from_f32_slice(&query, VectorPrecision::F16);
    let close_f16 = VectorData::from_f32_slice(&close, VectorPrecision::F16);
    let far_f16 = VectorData::from_f32_slice(&far, VectorPrecision::F16);

    let dist_close_f16 = cosine_similarity(&q_f16, &close_f16);
    let dist_far_f16 = cosine_similarity(&q_f16, &far_f16);

    // Ranking should be preserved
    assert!(
        dist_close_f32 > dist_far_f32,
        "F32: close should be more similar than far"
    );
    assert!(
        dist_close_f16 > dist_far_f16,
        "F16: ranking should be preserved"
    );
}

// =========================================================================
// Serialization tests
// =========================================================================

#[test]
fn test_vector_data_serialization() {
    let data = vec![0.1, 0.2, 0.3];

    for precision in [
        VectorPrecision::F32,
        VectorPrecision::F16,
        VectorPrecision::BF16,
    ] {
        let v = VectorData::from_f32_slice(&data, precision);
        let json = serde_json::to_string(&v).expect("serialize");
        let back: VectorData = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(v.precision(), back.precision());
        assert_eq!(v.len(), back.len());
    }
}

#[test]
fn test_precision_serialization() {
    for precision in [
        VectorPrecision::F32,
        VectorPrecision::F16,
        VectorPrecision::BF16,
    ] {
        let json = serde_json::to_string(&precision).expect("serialize");
        let back: VectorPrecision = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(precision, back);
    }
}

// =========================================================================
// Edge cases
// =========================================================================

#[test]
fn test_empty_vector() {
    let v = VectorData::from_f32_slice(&[], VectorPrecision::F16);
    assert!(v.is_empty());
    assert_eq!(v.len(), 0);
    assert_eq!(v.memory_size(), 0);
}

#[test]
fn test_large_vector_4096d() {
    let data = generate_test_vector(4096, 0.0);

    let v_f16 = VectorData::from_f32_slice(&data, VectorPrecision::F16);
    assert_eq!(v_f16.len(), 4096);
    assert_eq!(v_f16.memory_size(), 8192); // 8 KB

    // Verify conversion works
    let back = v_f16.to_f32_vec();
    assert_eq!(back.len(), 4096);
}

#[test]
fn test_from_impls() {
    let data = vec![0.1, 0.2, 0.3];

    // From Vec<f32>
    let v: VectorData = data.clone().into();
    assert_eq!(v.precision(), VectorPrecision::F32);

    // From &[f32]
    let v: VectorData = data.as_slice().into();
    assert_eq!(v.precision(), VectorPrecision::F32);
}
