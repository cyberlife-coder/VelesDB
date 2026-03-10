//! Scalar Quantization (SQ8) for memory-efficient vector storage.
//!
//! Implements 8-bit scalar quantization to reduce memory usage by 4x
//! while maintaining >95% recall accuracy. Includes both scalar and
//! SIMD-optimized distance functions.

use std::io;

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
            // SAFETY: Value is clamped to [0.0, 255.0] before cast, guaranteeing it fits in u8.
            // cast_sign_loss is safe because clamped value is always non-negative.
            // cast_possible_truncation is safe because clamped value is always <= 255.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            vector
                .iter()
                .map(|&v| {
                    let normalized = (v - min) * scale;
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

// =========================================================================
// Scalar distance functions
// =========================================================================

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
/// F-10: Computes quantized vector norm without full dequantization allocation.
/// Uses on-the-fly dequantization to accumulate norm squared, avoiding a
/// 3KB `Vec<f32>` allocation per call (for dim=768).
///
/// Note: For best accuracy, the query should be normalized.
#[must_use]
pub fn cosine_similarity_quantized(query: &[f32], quantized: &QuantizedVector) -> f32 {
    use crate::simd_native;

    let dot = dot_product_quantized(query, quantized);
    let query_norm = simd_native::norm_native(query);

    // F-10: Compute quantized norm without allocating a full f32 vector
    let quantized_norm = quantized_vector_norm(quantized);

    if query_norm < f32::EPSILON || quantized_norm < f32::EPSILON {
        return 0.0;
    }

    dot / (query_norm * quantized_norm)
}

/// Computes the L2 norm of a quantized vector without full dequantization.
///
/// F-10: Avoids allocating a `Vec<f32>` just to compute a norm.
/// Uses on-the-fly dequantization with 4-wide unrolling.
#[inline]
fn quantized_vector_norm(quantized: &QuantizedVector) -> f32 {
    let range = quantized.max - quantized.min;
    if range < f32::EPSILON {
        let value = quantized.min;
        #[allow(clippy::cast_precision_loss)]
        return value.abs() * (quantized.data.len() as f32).sqrt();
    }

    let scale = range / 255.0;
    let offset = quantized.min;
    let len = quantized.data.len();
    let chunks = len / 4;
    let remainder = len % 4;

    let mut sum0: f32 = 0.0;
    let mut sum1: f32 = 0.0;
    let mut sum2: f32 = 0.0;
    let mut sum3: f32 = 0.0;

    for i in 0..chunks {
        let base = i * 4;
        let d0 = f32::from(quantized.data[base]) * scale + offset;
        let d1 = f32::from(quantized.data[base + 1]) * scale + offset;
        let d2 = f32::from(quantized.data[base + 2]) * scale + offset;
        let d3 = f32::from(quantized.data[base + 3]) * scale + offset;
        sum0 += d0 * d0;
        sum1 += d1 * d1;
        sum2 += d2 * d2;
        sum3 += d3 * d3;
    }

    let base = chunks * 4;
    for i in 0..remainder {
        let d = f32::from(quantized.data[base + i]) * scale + offset;
        sum0 += d * d;
    }

    (sum0 + sum1 + sum2 + sum3).sqrt()
}

// =========================================================================
// SIMD-optimized distance functions for SQ8 quantized vectors
// =========================================================================

// F-11: Removed dead `use std::arch::x86_64::*` import — no intrinsics used in this file.

/// Dot product between f32 query and SQ8 quantized vector with 8-wide unrolling.
///
/// F-11: Renamed from `simd_dot_product_avx2` — this is an unrolled scalar
/// implementation, NOT actual AVX2 intrinsics. The 8-wide unrolling helps
/// LLVM auto-vectorize but does not guarantee SIMD execution.
#[must_use]
pub fn dot_product_quantized_simd(query: &[f32], quantized: &QuantizedVector) -> f32 {
    debug_assert_eq!(
        query.len(),
        quantized.data.len(),
        "Dimension mismatch in dot_product_quantized_simd"
    );

    let range = quantized.max - quantized.min;
    if range < f32::EPSILON {
        let value = quantized.min;
        return query.iter().sum::<f32>() * value;
    }

    let scale = range / 255.0;
    let offset = quantized.min;

    dot_product_dequant_unrolled_8(query, &quantized.data, scale, offset)
}

/// F-11: Honest name — unrolled scalar dequantize+dot, not intrinsics.
#[inline]
fn dot_product_dequant_unrolled_8(query: &[f32], data: &[u8], scale: f32, offset: f32) -> f32 {
    let len = query.len();
    let chunks = len / 8;
    let remainder = len % 8;

    let mut sum = 0.0f32;

    for i in 0..chunks {
        let base = i * 8;
        for j in 0..8 {
            let dequant = f32::from(data[base + j]) * scale + offset;
            sum += query[base + j] * dequant;
        }
    }

    let base = chunks * 8;
    for i in 0..remainder {
        let dequant = f32::from(data[base + i]) * scale + offset;
        sum += query[base + i] * dequant;
    }

    sum
}

/// SIMD-optimized squared Euclidean distance between f32 query and SQ8 vector.
#[must_use]
pub fn euclidean_squared_quantized_simd(query: &[f32], quantized: &QuantizedVector) -> f32 {
    debug_assert_eq!(
        query.len(),
        quantized.data.len(),
        "Dimension mismatch in euclidean_squared_quantized_simd"
    );

    let range = quantized.max - quantized.min;
    if range < f32::EPSILON {
        let value = quantized.min;
        return query.iter().map(|&q| (q - value).powi(2)).sum();
    }

    let scale = range / 255.0;
    let offset = quantized.min;

    // Optimized loop with manual unrolling
    let len = query.len();
    let chunks = len / 4;
    let remainder = len % 4;
    let mut sum = 0.0f32;

    for i in 0..chunks {
        let base = i * 4;
        let d0 = f32::from(quantized.data[base]) * scale + offset;
        let d1 = f32::from(quantized.data[base + 1]) * scale + offset;
        let d2 = f32::from(quantized.data[base + 2]) * scale + offset;
        let d3 = f32::from(quantized.data[base + 3]) * scale + offset;

        let diff0 = query[base] - d0;
        let diff1 = query[base + 1] - d1;
        let diff2 = query[base + 2] - d2;
        let diff3 = query[base + 3] - d3;

        sum += diff0 * diff0 + diff1 * diff1 + diff2 * diff2 + diff3 * diff3;
    }

    let base = chunks * 4;
    for i in 0..remainder {
        let dequant = f32::from(quantized.data[base + i]) * scale + offset;
        let diff = query[base + i] - dequant;
        sum += diff * diff;
    }

    sum
}

/// SIMD-optimized cosine similarity between f32 query and SQ8 vector.
///
/// F-10: Uses `quantized_vector_norm` for allocation-free norm computation,
/// consistent with the non-SIMD `cosine_similarity_quantized`.
#[must_use]
pub fn cosine_similarity_quantized_simd(query: &[f32], quantized: &QuantizedVector) -> f32 {
    use crate::simd_native;

    let dot = dot_product_quantized_simd(query, quantized);
    let query_norm = simd_native::norm_native(query);

    // F-10: Compute quantized norm without allocating a full f32 vector
    let quantized_norm = quantized_vector_norm(quantized);

    if query_norm < f32::EPSILON || quantized_norm < f32::EPSILON {
        return 0.0;
    }

    dot / (query_norm * quantized_norm)
}
