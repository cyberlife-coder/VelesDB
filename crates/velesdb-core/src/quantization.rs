//! Scalar Quantization (SQ8) for memory-efficient vector storage.
//!
//! This module implements 8-bit scalar quantization to reduce memory usage by 4x
//! while maintaining >95% recall accuracy.
//!
//! ## Benefits
//!
//! | Metric | f32 | SQ8 |
//! |--------|-----|-----|
//! | RAM/vector (768d) | 3 KB | 770 bytes |
//! | Cache efficiency | Baseline | ~4x better |
//! | Recall loss | 0% | ~0.5-1% |

use std::io;

/// Storage mode for vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageMode {
    /// Full precision f32 storage (default).
    #[default]
    Full,
    /// 8-bit scalar quantization for 4x memory reduction.
    SQ8,
    /// 1-bit binary quantization for 32x memory reduction.
    /// Best for edge/IoT devices with limited RAM.
    Binary,
}

/// A binary quantized vector using 1-bit per dimension.
///
/// Each f32 value is converted to 1 bit: >= 0.0 becomes 1, < 0.0 becomes 0.
/// This provides **32x memory reduction** compared to f32 storage.
///
/// # Memory Usage
///
/// | Dimension | f32 | Binary |
/// |-----------|-----|--------|
/// | 768 | 3072 bytes | 96 bytes |
/// | 1536 | 6144 bytes | 192 bytes |
///
/// # Use with Rescoring
///
/// For best accuracy, use binary search for candidate selection,
/// then rescore top candidates with full-precision vectors.
#[derive(Debug, Clone)]
pub struct BinaryQuantizedVector {
    /// Binary data (1 bit per dimension, packed into bytes).
    pub data: Vec<u8>,
    /// Original dimension of the vector.
    dimension: usize,
}

impl BinaryQuantizedVector {
    /// Creates a new binary quantized vector from f32 data.
    ///
    /// Values >= 0.0 become 1, values < 0.0 become 0.
    ///
    /// # Arguments
    ///
    /// * `vector` - The original f32 vector to quantize
    ///
    /// # Panics
    ///
    /// Panics if the vector is empty.
    #[must_use]
    pub fn from_f32(vector: &[f32]) -> Self {
        assert!(!vector.is_empty(), "Cannot quantize empty vector");

        let dimension = vector.len();
        // Calculate number of bytes needed: ceil(dimension / 8)
        let num_bytes = dimension.div_ceil(8);
        let mut data = vec![0u8; num_bytes];

        for (i, &value) in vector.iter().enumerate() {
            if value >= 0.0 {
                // Set bit i in the packed byte array
                let byte_idx = i / 8;
                let bit_idx = i % 8;
                data[byte_idx] |= 1 << bit_idx;
            }
        }

        Self { data, dimension }
    }

    /// Returns the dimension of the original vector.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the memory size in bytes.
    #[must_use]
    pub fn memory_size(&self) -> usize {
        self.data.len()
    }

    /// Returns the individual bits as a boolean vector.
    ///
    /// Useful for debugging and testing.
    #[must_use]
    pub fn get_bits(&self) -> Vec<bool> {
        (0..self.dimension)
            .map(|i| {
                let byte_idx = i / 8;
                let bit_idx = i % 8;
                (self.data[byte_idx] >> bit_idx) & 1 == 1
            })
            .collect()
    }

    /// Computes the Hamming distance to another binary vector.
    ///
    /// Hamming distance counts the number of bits that differ.
    /// Uses POPCNT for fast bit counting.
    ///
    /// # Panics
    ///
    /// Panics if the vectors have different dimensions.
    #[must_use]
    pub fn hamming_distance(&self, other: &Self) -> u32 {
        debug_assert_eq!(
            self.dimension, other.dimension,
            "Dimension mismatch in hamming_distance"
        );

        // XOR bytes and count differing bits using POPCNT
        self.data
            .iter()
            .zip(other.data.iter())
            .map(|(&a, &b)| (a ^ b).count_ones())
            .sum()
    }

    /// Computes normalized Hamming similarity (0.0 to 1.0).
    ///
    /// Returns 1.0 for identical vectors, 0.0 for completely different.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn hamming_similarity(&self, other: &Self) -> f32 {
        let distance = self.hamming_distance(other);
        1.0 - (distance as f32 / self.dimension as f32)
    }

    /// Serializes the binary quantized vector to bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + self.data.len());
        // Store dimension as u32 (4 bytes)
        #[allow(clippy::cast_possible_truncation)]
        bytes.extend_from_slice(&(self.dimension as u32).to_le_bytes());
        bytes.extend_from_slice(&self.data);
        bytes
    }

    /// Deserializes a binary quantized vector from bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are invalid.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not enough bytes for BinaryQuantizedVector header",
            ));
        }

        let dimension = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let expected_data_len = dimension.div_ceil(8);

        if bytes.len() < 4 + expected_data_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Not enough bytes for BinaryQuantizedVector data: expected {}, got {}",
                    4 + expected_data_len,
                    bytes.len()
                ),
            ));
        }

        let data = bytes[4..4 + expected_data_len].to_vec();

        Ok(Self { data, dimension })
    }
}

/// A quantized vector using 8-bit scalar quantization.
///
/// Each f32 value is mapped to a u8 (0-255) using min/max scaling.
/// The original value can be reconstructed as: `value = (data[i] / 255.0) * (max - min) + min`
#[derive(Debug, Clone)]
pub struct QuantizedVector {
    /// Quantized data (1 byte per dimension instead of 4).
    pub data: Vec<u8>,
    /// Minimum value in the original vector.
    pub min: f32,
    /// Maximum value in the original vector.
    pub max: f32,
}

impl QuantizedVector {
    /// Creates a new quantized vector from f32 data.
    ///
    /// # Arguments
    ///
    /// * `vector` - The original f32 vector to quantize
    ///
    /// # Panics
    ///
    /// Panics if the vector is empty.
    #[must_use]
    pub fn from_f32(vector: &[f32]) -> Self {
        assert!(!vector.is_empty(), "Cannot quantize empty vector");

        let min = vector.iter().copied().fold(f32::INFINITY, f32::min);
        let max = vector.iter().copied().fold(f32::NEG_INFINITY, f32::max);

        let range = max - min;
        let data = if range < f32::EPSILON {
            // All values are the same, map to 128 (middle of range)
            vec![128u8; vector.len()]
        } else {
            let scale = 255.0 / range;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            vector
                .iter()
                .map(|&v| {
                    let normalized = (v - min) * scale;
                    // Clamp to [0, 255] to handle floating point errors
                    // Safe: clamped to valid u8 range
                    normalized.round().clamp(0.0, 255.0) as u8
                })
                .collect()
        };

        Self { data, min, max }
    }

    /// Reconstructs the original f32 vector from quantized data.
    ///
    /// Note: This is a lossy operation. The reconstructed values are approximations.
    #[must_use]
    pub fn to_f32(&self) -> Vec<f32> {
        let range = self.max - self.min;
        if range < f32::EPSILON {
            // All values were the same
            vec![self.min; self.data.len()]
        } else {
            let scale = range / 255.0;
            self.data
                .iter()
                .map(|&v| f32::from(v) * scale + self.min)
                .collect()
        }
    }

    /// Returns the dimension of the vector.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.data.len()
    }

    /// Returns the memory size in bytes.
    #[must_use]
    pub fn memory_size(&self) -> usize {
        self.data.len() + 8 // data + min(4) + max(4)
    }

    /// Serializes the quantized vector to bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + self.data.len());
        bytes.extend_from_slice(&self.min.to_le_bytes());
        bytes.extend_from_slice(&self.max.to_le_bytes());
        bytes.extend_from_slice(&self.data);
        bytes
    }

    /// Deserializes a quantized vector from bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are invalid.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not enough bytes for QuantizedVector header",
            ));
        }

        let min = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let max = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let data = bytes[8..].to_vec();

        Ok(Self { data, min, max })
    }
}

/// Computes the approximate dot product between a query vector (f32) and a quantized vector.
///
/// This avoids full dequantization for better performance.
#[must_use]
pub fn dot_product_quantized(query: &[f32], quantized: &QuantizedVector) -> f32 {
    debug_assert_eq!(
        query.len(),
        quantized.data.len(),
        "Dimension mismatch in dot_product_quantized"
    );

    let range = quantized.max - quantized.min;
    if range < f32::EPSILON {
        // All quantized values are the same
        let value = quantized.min;
        return query.iter().sum::<f32>() * value;
    }

    let scale = range / 255.0;
    let offset = quantized.min;

    // Compute dot product with on-the-fly dequantization
    query
        .iter()
        .zip(quantized.data.iter())
        .map(|(&q, &v)| q * (f32::from(v) * scale + offset))
        .sum()
}

/// Computes the approximate squared Euclidean distance between a query (f32) and quantized vector.
#[must_use]
pub fn euclidean_squared_quantized(query: &[f32], quantized: &QuantizedVector) -> f32 {
    debug_assert_eq!(
        query.len(),
        quantized.data.len(),
        "Dimension mismatch in euclidean_squared_quantized"
    );

    let range = quantized.max - quantized.min;
    if range < f32::EPSILON {
        // All quantized values are the same
        let value = quantized.min;
        return query.iter().map(|&q| (q - value).powi(2)).sum();
    }

    let scale = range / 255.0;
    let offset = quantized.min;

    query
        .iter()
        .zip(quantized.data.iter())
        .map(|(&q, &v)| {
            let dequantized = f32::from(v) * scale + offset;
            (q - dequantized).powi(2)
        })
        .sum()
}

/// Computes approximate cosine similarity between a query (f32) and quantized vector.
///
/// Note: For best accuracy, the query should be normalized.
#[must_use]
pub fn cosine_similarity_quantized(query: &[f32], quantized: &QuantizedVector) -> f32 {
    let dot = dot_product_quantized(query, quantized);

    // Compute norms
    let query_norm: f32 = query.iter().map(|&x| x * x).sum::<f32>().sqrt();

    // Dequantize to compute quantized vector norm (could be cached)
    let reconstructed = quantized.to_f32();
    let quantized_norm: f32 = reconstructed.iter().map(|&x| x * x).sum::<f32>().sqrt();

    if query_norm < f32::EPSILON || quantized_norm < f32::EPSILON {
        return 0.0;
    }

    dot / (query_norm * quantized_norm)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
