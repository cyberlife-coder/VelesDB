use super::*;

#[test]
fn test_vector_data_f16_roundtrip() {
    let data = vec![1.0, 2.0, 3.0];
    let v = VectorData::from_f32_slice(&data, VectorPrecision::F16);
    let result = v.to_f32_vec();
    for (a, b) in data.iter().zip(result.iter()) {
        assert!((a - b).abs() < 0.01);
    }
}

#[test]
fn test_cosine_similarity_identical() {
    let v1 = VectorData::from_f32_slice(&[1.0, 0.0, 0.0], VectorPrecision::F32);
    let v2 = VectorData::from_f32_slice(&[1.0, 0.0, 0.0], VectorPrecision::F32);
    let sim = cosine_similarity(&v1, &v2);
    assert!((sim - 1.0).abs() < 1e-5);
}

#[test]
fn test_cosine_similarity_orthogonal() {
    let v1 = VectorData::from_f32_slice(&[1.0, 0.0, 0.0], VectorPrecision::F32);
    let v2 = VectorData::from_f32_slice(&[0.0, 1.0, 0.0], VectorPrecision::F32);
    let sim = cosine_similarity(&v1, &v2);
    assert!(sim.abs() < 1e-5);
}

#[test]
fn test_euclidean_distance_identical() {
    let v1 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
    let v2 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
    let dist = euclidean_distance(&v1, &v2);
    assert!(dist.abs() < 1e-5);
}

#[test]
fn test_euclidean_distance_345() {
    let v1 = VectorData::from_f32_slice(&[0.0, 0.0], VectorPrecision::F32);
    let v2 = VectorData::from_f32_slice(&[3.0, 4.0], VectorPrecision::F32);
    let dist = euclidean_distance(&v1, &v2);
    assert!((dist - 5.0).abs() < 1e-5);
}

#[test]
fn test_norm_squared_f32() {
    let v = VectorData::from_f32_slice(&[3.0, 4.0], VectorPrecision::F32);
    let norm = norm_squared(&v);
    assert!((norm - 25.0).abs() < 1e-5);
}

#[test]
fn test_cosine_similarity_f16_vs_f32() {
    let v1 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F16);
    let v2 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
    let sim = cosine_similarity(&v1, &v2);
    assert!((sim - 1.0).abs() < 0.01);
}

#[test]
fn test_cosine_similarity_is_clamped_to_unit_interval() {
    // Mixed precision path (non-F32/F32) must respect cosine bounds.
    let v1 = VectorData::from_f32_slice(&[1.0, 1.0, 1.0, 1.0], VectorPrecision::F16);
    let v2 = VectorData::from_f32_slice(&[1.0, 1.0, 1.0, 1.0], VectorPrecision::BF16);
    let sim = cosine_similarity(&v1, &v2);
    assert!(
        (-1.0..=1.0).contains(&sim),
        "cosine similarity must be clamped to [-1, 1], got {sim}"
    );
}
