//! Tests for vector_bytes module.

use super::vector_bytes::{bytes_to_vector, vector_to_bytes};

#[test]
fn test_vector_to_bytes_empty() {
    let vector: Vec<f32> = vec![];
    let bytes = vector_to_bytes(&vector);
    assert!(bytes.is_empty());
}

#[test]
fn test_vector_to_bytes_single_element() {
    let vector = vec![1.0f32];
    let bytes = vector_to_bytes(&vector);
    assert_eq!(bytes.len(), 4); // f32 = 4 bytes
}

#[test]
fn test_vector_to_bytes_multiple_elements() {
    let vector = vec![1.0f32, 2.0, 3.0, 4.0];
    let bytes = vector_to_bytes(&vector);
    assert_eq!(bytes.len(), 16); // 4 * 4 bytes
}

#[test]
fn test_bytes_to_vector_roundtrip() {
    let original = vec![1.5f32, -2.5, 3.125, 0.0];
    let bytes = vector_to_bytes(&original);
    let recovered = bytes_to_vector(bytes, original.len());
    assert_eq!(original, recovered);
}

#[test]
fn test_bytes_to_vector_dimension_1() {
    let vector = vec![42.0f32];
    let bytes = vector_to_bytes(&vector);
    let recovered = bytes_to_vector(bytes, 1);
    assert_eq!(vector, recovered);
}

#[test]
fn test_bytes_to_vector_high_dimension() {
    let vector: Vec<f32> = (0..128).map(|i| i as f32 * 0.1).collect();
    let bytes = vector_to_bytes(&vector);
    let recovered = bytes_to_vector(bytes, 128);
    assert_eq!(vector, recovered);
}

#[test]
#[should_panic(expected = "buffer too small")]
fn test_bytes_to_vector_buffer_underflow_panics() {
    let small_buffer = [0u8; 4]; // Only 4 bytes
    bytes_to_vector(&small_buffer, 4); // Expects 16 bytes (4 * sizeof(f32))
}

#[test]
#[should_panic(expected = "buffer too small")]
fn test_bytes_to_vector_empty_buffer_panics() {
    let empty_buffer: [u8; 0] = [];
    bytes_to_vector(&empty_buffer, 1); // Expects 4 bytes
}

#[test]
fn test_bytes_to_vector_exact_size() {
    let bytes = [0u8; 12]; // Exactly 3 * 4 bytes
    let vector = bytes_to_vector(&bytes, 3);
    assert_eq!(vector.len(), 3);
    assert!(vector.iter().all(|&v| v == 0.0));
}

#[test]
fn test_vector_to_bytes_preserves_special_values() {
    let vector = vec![f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0];
    let bytes = vector_to_bytes(&vector);
    let recovered = bytes_to_vector(bytes, 4);

    assert!(recovered[0].is_infinite() && recovered[0].is_sign_positive());
    assert!(recovered[1].is_infinite() && recovered[1].is_sign_negative());
    assert!((recovered[2] - 0.0).abs() < f32::EPSILON);
    assert!((recovered[3] - 0.0).abs() < f32::EPSILON); // -0.0 == 0.0 in Rust
}
