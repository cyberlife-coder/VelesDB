//! Tests for quantization module

use crate::quantization::*;

// =========================================================================
// TDD Tests for SIMD Quantized Distance Functions
// =========================================================================

#[test]
fn test_dot_product_quantized_simd_simple() {
    let query = vec![1.0, 0.0, 0.0];
    let vector = vec![1.0, 0.0, 0.0];
    let quantized = QuantizedVector::from_f32(&vector);

    let result = dot_product_quantized_simd(&query, &quantized);
    assert!(
        (result - 1.0).abs() < 0.1,
        "Result {result} not close to 1.0"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_dot_product_quantized_simd_768d() {
    let dimension = 768;
    let query: Vec<f32> = (0..dimension).map(|i| (i as f32) / 1000.0).collect();
    let vector: Vec<f32> = (0..dimension).map(|i| (i as f32) / 1000.0).collect();
    let quantized = QuantizedVector::from_f32(&vector);

    let scalar = dot_product_quantized(&query, &quantized);
    let simd = dot_product_quantized_simd(&query, &quantized);

    // Results should be very close
    let rel_error = ((scalar - simd) / scalar).abs();
    assert!(rel_error < 0.01, "Relative error {rel_error} too high");
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_euclidean_squared_quantized_simd_768d() {
    let dimension = 768;
    let query: Vec<f32> = (0..dimension).map(|i| (i as f32) / 1000.0).collect();
    let vector: Vec<f32> = (0..dimension).map(|i| ((i + 10) as f32) / 1000.0).collect();
    let quantized = QuantizedVector::from_f32(&vector);

    let scalar = euclidean_squared_quantized(&query, &quantized);
    let simd = euclidean_squared_quantized_simd(&query, &quantized);

    let rel_error = ((scalar - simd) / scalar).abs();
    assert!(rel_error < 0.01, "Relative error {rel_error} too high");
}

#[test]
fn test_cosine_similarity_quantized_simd_identical() {
    let vector = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let quantized = QuantizedVector::from_f32(&vector);

    let similarity = cosine_similarity_quantized_simd(&vector, &quantized);
    assert!(
        (similarity - 1.0).abs() < 0.05,
        "Similarity {similarity} not close to 1.0"
    );
}

// =========================================================================
// TDD Tests for QuantizedVector
// =========================================================================

#[test]
fn test_quantize_simple_vector() {
    // Arrange
    let vector = vec![0.0, 0.5, 1.0];

    // Act
    let quantized = QuantizedVector::from_f32(&vector);

    // Assert
    assert_eq!(quantized.dimension(), 3);
    assert!((quantized.min - 0.0).abs() < f32::EPSILON);
    assert!((quantized.max - 1.0).abs() < f32::EPSILON);
    assert_eq!(quantized.data[0], 0); // 0.0 -> 0
    assert_eq!(quantized.data[1], 128); // 0.5 -> ~128
    assert_eq!(quantized.data[2], 255); // 1.0 -> 255
}

#[test]
fn test_quantize_negative_values() {
    // Arrange
    let vector = vec![-1.0, 0.0, 1.0];

    // Act
    let quantized = QuantizedVector::from_f32(&vector);

    // Assert
    assert!((quantized.min - (-1.0)).abs() < f32::EPSILON);
    assert!((quantized.max - 1.0).abs() < f32::EPSILON);
    assert_eq!(quantized.data[0], 0); // -1.0 -> 0
    assert_eq!(quantized.data[1], 128); // 0.0 -> ~128
    assert_eq!(quantized.data[2], 255); // 1.0 -> 255
}

#[test]
fn test_quantize_constant_vector() {
    // Arrange - all values the same
    let vector = vec![0.5, 0.5, 0.5];

    // Act
    let quantized = QuantizedVector::from_f32(&vector);

    // Assert - should handle gracefully
    assert_eq!(quantized.dimension(), 3);
    // All values should be middle (128)
    for &v in &quantized.data {
        assert_eq!(v, 128);
    }
}

#[test]
fn test_dequantize_roundtrip() {
    // Arrange
    let original = vec![0.1, 0.5, 0.9, -0.3, 0.0];

    // Act
    let quantized = QuantizedVector::from_f32(&original);
    let reconstructed = quantized.to_f32();

    // Assert - reconstructed should be close to original
    assert_eq!(reconstructed.len(), original.len());
    for (orig, recon) in original.iter().zip(reconstructed.iter()) {
        let error = (orig - recon).abs();
        // Max error should be less than range/255
        let max_error = (quantized.max - quantized.min) / 255.0;
        assert!(
            error <= max_error + f32::EPSILON,
            "Error {error} exceeds max {max_error}"
        );
    }
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_memory_reduction() {
    // Arrange
    let dimension = 768;
    let vector: Vec<f32> = (0..dimension)
        .map(|i| i as f32 / dimension as f32)
        .collect();

    // Act
    let quantized = QuantizedVector::from_f32(&vector);

    // Assert
    let f32_size = dimension * 4; // 3072 bytes
    let sq8_size = quantized.memory_size(); // 768 + 8 = 776 bytes

    assert_eq!(f32_size, 3072);
    assert_eq!(sq8_size, 776);
    // ~4x reduction
    #[allow(clippy::cast_precision_loss)]
    let ratio = f32_size as f32 / sq8_size as f32;
    assert!(ratio > 3.9);
}

#[test]
fn test_serialization_roundtrip() {
    // Arrange
    let vector = vec![0.1, 0.5, 0.9, -0.3];
    let quantized = QuantizedVector::from_f32(&vector);

    // Act
    let bytes = quantized.to_bytes();
    let deserialized = QuantizedVector::from_bytes(&bytes).unwrap();

    // Assert
    assert!((deserialized.min - quantized.min).abs() < f32::EPSILON);
    assert!((deserialized.max - quantized.max).abs() < f32::EPSILON);
    assert_eq!(deserialized.data, quantized.data);
}

#[test]
fn test_from_bytes_invalid() {
    // Arrange - too few bytes
    let bytes = vec![0u8; 5];

    // Act
    let result = QuantizedVector::from_bytes(&bytes);

    // Assert
    assert!(result.is_err());
}

// =========================================================================
// TDD Tests for Distance Functions
// =========================================================================

#[test]
fn test_dot_product_quantized_simple() {
    // Arrange
    let query = vec![1.0, 0.0, 0.0];
    let vector = vec![1.0, 0.0, 0.0];
    let quantized = QuantizedVector::from_f32(&vector);

    // Act
    let dot = dot_product_quantized(&query, &quantized);

    // Assert - should be close to 1.0
    assert!(
        (dot - 1.0).abs() < 0.1,
        "Dot product {dot} not close to 1.0"
    );
}

#[test]
fn test_dot_product_quantized_orthogonal() {
    // Arrange
    let query = vec![1.0, 0.0, 0.0];
    let vector = vec![0.0, 1.0, 0.0];
    let quantized = QuantizedVector::from_f32(&vector);

    // Act
    let dot = dot_product_quantized(&query, &quantized);

    // Assert - should be close to 0
    assert!(dot.abs() < 0.1, "Dot product {dot} not close to 0");
}

#[test]
fn test_euclidean_squared_quantized() {
    // Arrange
    let query = vec![0.0, 0.0, 0.0];
    let vector = vec![1.0, 0.0, 0.0];
    let quantized = QuantizedVector::from_f32(&vector);

    // Act
    let dist = euclidean_squared_quantized(&query, &quantized);

    // Assert - should be close to 1.0
    assert!(
        (dist - 1.0).abs() < 0.1,
        "Euclidean squared {dist} not close to 1.0"
    );
}

#[test]
fn test_euclidean_squared_quantized_same_point() {
    // Arrange
    let vector = vec![0.5, 0.5, 0.5];
    let quantized = QuantizedVector::from_f32(&vector);

    // Act
    let dist = euclidean_squared_quantized(&vector, &quantized);

    // Assert - distance to self should be ~0
    assert!(dist < 0.01, "Distance to self {dist} should be ~0");
}

#[test]
fn test_cosine_similarity_quantized_identical() {
    // Arrange
    let vector = vec![1.0, 2.0, 3.0];
    let quantized = QuantizedVector::from_f32(&vector);

    // Act
    let similarity = cosine_similarity_quantized(&vector, &quantized);

    // Assert - similarity to self should be ~1.0
    assert!(
        (similarity - 1.0).abs() < 0.05,
        "Cosine similarity to self {similarity} not close to 1.0"
    );
}

#[test]
fn test_cosine_similarity_quantized_opposite() {
    // Arrange
    let query = vec![1.0, 0.0, 0.0];
    let vector = vec![-1.0, 0.0, 0.0];
    let quantized = QuantizedVector::from_f32(&vector);

    // Act
    let similarity = cosine_similarity_quantized(&query, &quantized);

    // Assert - opposite vectors should have similarity ~-1.0
    assert!(
        (similarity - (-1.0)).abs() < 0.1,
        "Cosine similarity {similarity} not close to -1.0"
    );
}

// =========================================================================
// TDD Tests for Recall Accuracy
// =========================================================================

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_recall_accuracy_high_dimension() {
    // Arrange - simulate real embedding vectors
    let dimension = 768;
    let num_vectors = 100;

    // Generate random-ish vectors
    let vectors: Vec<Vec<f32>> = (0..num_vectors)
        .map(|i| {
            (0..dimension)
                .map(|j| {
                    let x = ((i * 7 + j * 13) % 1000) as f32 / 1000.0;
                    x * 2.0 - 1.0 // Range [-1, 1]
                })
                .collect()
        })
        .collect();

    // Quantize all vectors
    let quantized: Vec<QuantizedVector> = vectors
        .iter()
        .map(|v| QuantizedVector::from_f32(v))
        .collect();

    // Query vector
    let query = &vectors[0];

    // Act - compute distances with both methods
    let mut f32_distances: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let dot: f32 = query.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
            (i, dot)
        })
        .collect();

    let mut sq8_distances: Vec<(usize, f32)> = quantized
        .iter()
        .enumerate()
        .map(|(i, q)| (i, dot_product_quantized(query, q)))
        .collect();

    // Sort by distance (descending for dot product)
    f32_distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    sq8_distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // Assert - check recall@10
    let k = 10;
    let f32_top_k: std::collections::HashSet<usize> =
        f32_distances.iter().take(k).map(|(i, _)| *i).collect();
    let sq8_top_k: std::collections::HashSet<usize> =
        sq8_distances.iter().take(k).map(|(i, _)| *i).collect();

    let recall = f32_top_k.intersection(&sq8_top_k).count() as f32 / k as f32;

    assert!(
        recall >= 0.8,
        "Recall@{k} is {recall}, expected >= 0.8 (80%)"
    );
}

#[test]
fn test_storage_mode_enum() {
    // Arrange & Act
    let full = StorageMode::Full;
    let sq8 = StorageMode::SQ8;
    let binary = StorageMode::Binary;
    let default = StorageMode::default();

    // Assert
    assert_eq!(full, StorageMode::Full);
    assert_eq!(sq8, StorageMode::SQ8);
    assert_eq!(binary, StorageMode::Binary);
    assert_eq!(default, StorageMode::Full);
    assert_ne!(full, sq8);
    assert_ne!(sq8, binary);
}

// =========================================================================
// TDD Tests for BinaryQuantizedVector
// =========================================================================

#[test]
fn test_binary_quantize_simple_vector() {
    // Arrange - positive values become 1, negative become 0
    let vector = vec![-1.0, 0.5, -0.5, 1.0];

    // Act
    let binary = BinaryQuantizedVector::from_f32(&vector);

    // Assert
    assert_eq!(binary.dimension(), 4);
    // Bit pattern: 0, 1, 0, 1 = 0b0101 = 5 (reversed in byte)
    // Actually stored as: bit 0 = vec[0], bit 1 = vec[1], etc.
    // -1.0 -> 0, 0.5 -> 1, -0.5 -> 0, 1.0 -> 1
    // Bits: 0b1010 when read left to right, but stored as 0b0101
    assert_eq!(binary.data.len(), 1); // 4 bits fits in 1 byte
}

#[test]
fn test_binary_quantize_768d_memory() {
    // Arrange - simulate real embedding dimension
    let vector: Vec<f32> = (0..768)
        .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
        .collect();

    // Act
    let binary = BinaryQuantizedVector::from_f32(&vector);

    // Assert - 768 bits = 96 bytes
    assert_eq!(binary.dimension(), 768);
    assert_eq!(binary.data.len(), 96); // 768 / 8 = 96 bytes

    // Memory comparison:
    // f32: 768 * 4 = 3072 bytes
    // Binary: 96 bytes
    // Ratio: 32x reduction!
    let f32_size = 768 * 4;
    let binary_size = binary.memory_size();
    assert_eq!(binary_size, 96);
    #[allow(clippy::cast_precision_loss)]
    let ratio = f32_size as f32 / binary_size as f32;
    assert!(ratio >= 32.0, "Expected 32x reduction, got {ratio}x");
}

#[test]
fn test_binary_quantize_threshold_at_zero() {
    // Arrange - test threshold behavior
    let vector = vec![0.0, 0.001, -0.001, f32::EPSILON];

    // Act
    let binary = BinaryQuantizedVector::from_f32(&vector);

    // Assert - 0.0 and positive become 1, negative become 0
    // Using >= 0.0 as threshold
    let bits = binary.get_bits();
    assert!(bits[0], "0.0 should be 1");
    assert!(bits[1], "0.001 should be 1");
    assert!(!bits[2], "-0.001 should be 0");
    assert!(bits[3], "EPSILON should be 1");
}

#[test]
fn test_binary_hamming_distance_identical() {
    // Arrange
    let vector = vec![0.5, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5, -0.5];
    let binary = BinaryQuantizedVector::from_f32(&vector);

    // Act
    let distance = binary.hamming_distance(&binary);

    // Assert - identical vectors have 0 distance
    assert_eq!(distance, 0);
}

#[test]
fn test_binary_hamming_distance_opposite() {
    // Arrange
    let v1 = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let v2 = vec![-1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0];
    let b1 = BinaryQuantizedVector::from_f32(&v1);
    let b2 = BinaryQuantizedVector::from_f32(&v2);

    // Act
    let distance = b1.hamming_distance(&b2);

    // Assert - all bits different = 8 distance
    assert_eq!(distance, 8);
}

#[test]
fn test_binary_hamming_distance_half_different() {
    // Arrange - half the bits differ
    let v1 = vec![1.0, 1.0, 1.0, 1.0, -1.0, -1.0, -1.0, -1.0];
    let v2 = vec![1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];
    let b1 = BinaryQuantizedVector::from_f32(&v1);
    let b2 = BinaryQuantizedVector::from_f32(&v2);

    // Act
    let distance = b1.hamming_distance(&b2);

    // Assert - 4 bits differ
    assert_eq!(distance, 4);
}

#[test]
fn test_binary_serialization_roundtrip() {
    // Arrange
    let vector: Vec<f32> = (0..768)
        .map(|i| if i % 3 == 0 { 0.5 } else { -0.5 })
        .collect();
    let binary = BinaryQuantizedVector::from_f32(&vector);

    // Act
    let bytes = binary.to_bytes();
    let deserialized = BinaryQuantizedVector::from_bytes(&bytes).unwrap();

    // Assert
    assert_eq!(deserialized.dimension(), binary.dimension());
    assert_eq!(deserialized.data, binary.data);
    assert_eq!(deserialized.hamming_distance(&binary), 0);
}

#[test]
fn test_binary_from_bytes_invalid() {
    // Arrange - too few bytes for header
    let bytes = vec![0u8; 3];

    // Act
    let result = BinaryQuantizedVector::from_bytes(&bytes);

    // Assert
    assert!(result.is_err());
}
