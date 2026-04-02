//! Binary quantization (1-bit per dimension) for extreme memory reduction.
//!
//! Each f32 value is converted to 1 bit: >= 0.0 becomes 1, < 0.0 becomes 0.
//! This provides **32x memory reduction** compared to f32 storage.

use std::io;

use super::codec_helpers::{serialize_with_header, validate_and_split_header};
use super::QuantizationCodec;

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
}

/// Binary header: `[dimension: u32 LE]` = 4 bytes.
const BINARY_HEADER_SIZE: usize = 4;

impl QuantizationCodec for BinaryQuantizedVector {
    /// # Panics
    ///
    /// Panics if dimension exceeds `u32::MAX` (4,294,967,295).
    fn to_bytes(&self) -> Vec<u8> {
        assert!(
            u32::try_from(self.dimension).is_ok(),
            "BinaryQuantizedVector dimension {} exceeds u32::MAX for serialization",
            self.dimension
        );

        // Reason: dimension validated above to fit in u32
        #[allow(clippy::cast_possible_truncation)]
        let header = (self.dimension as u32).to_le_bytes();
        serialize_with_header(&header, &self.data)
    }

    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let (header, payload) =
            validate_and_split_header(bytes, BINARY_HEADER_SIZE, "BinaryQuantizedVector")?;

        #[allow(clippy::cast_possible_truncation)]
        // Reason: u32 always fits in usize on 32-bit and 64-bit platforms
        let dimension = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let expected_data_len = dimension.div_ceil(8);

        if payload.len() < expected_data_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Not enough bytes for BinaryQuantizedVector data: expected {}, got {}",
                    BINARY_HEADER_SIZE + expected_data_len,
                    bytes.len()
                ),
            ));
        }

        let data = payload[..expected_data_len].to_vec();

        Ok(Self { data, dimension })
    }
}
