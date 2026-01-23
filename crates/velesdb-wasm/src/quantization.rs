//! Quantization helpers for vector storage.
//!
//! Provides SQ8 (8-bit scalar quantization) and Binary (1-bit) quantization
//! for memory-efficient vector storage.

/// SQ8 quantization parameters for a single vector.
#[derive(Debug, Clone, Copy)]
pub struct Sq8Params {
    pub min: f32,
    pub scale: f32,
}

/// Computes SQ8 quantization parameters (min, scale) for a vector.
#[must_use]
pub fn compute_sq8_params(vector: &[f32]) -> Sq8Params {
    let (min, max) = vector
        .iter()
        .fold((f32::MAX, f32::MIN), |(min, max), &v| (min.min(v), max.max(v)));

    let scale = if (max - min).abs() < 1e-10 {
        1.0
    } else {
        255.0 / (max - min)
    };

    Sq8Params { min, scale }
}

/// Quantizes a vector to SQ8 format using pre-computed parameters.
///
/// Each f32 value is mapped to u8 range [0, 255].
pub fn quantize_sq8(vector: &[f32], params: Sq8Params, output: &mut Vec<u8>) {
    for &v in vector {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let quantized = ((v - params.min) * params.scale).round().clamp(0.0, 255.0) as u8;
        output.push(quantized);
    }
}

/// Dequantizes SQ8 data back to f32.
#[must_use]
pub fn dequantize_sq8(data: &[u8], params: Sq8Params) -> Vec<f32> {
    data.iter()
        .map(|&q| (f32::from(q) / params.scale) + params.min)
        .collect()
}

/// Packs a vector into binary format (1 bit per dimension).
///
/// Positive values (> 0) become 1, others become 0.
/// Bits are packed 8 per byte, LSB first.
pub fn pack_binary(vector: &[f32], dimension: usize, output: &mut Vec<u8>) {
    let bytes_needed = dimension.div_ceil(8);
    for byte_idx in 0..bytes_needed {
        let mut byte = 0u8;
        for bit in 0..8 {
            let dim_idx = byte_idx * 8 + bit;
            if dim_idx < dimension && vector[dim_idx] > 0.0 {
                byte |= 1 << bit;
            }
        }
        output.push(byte);
    }
}

/// Unpacks binary data back to f32 vector.
///
/// Returns 1.0 for set bits, 0.0 for unset bits.
#[must_use]
pub fn unpack_binary(data: &[u8], dimension: usize) -> Vec<f32> {
    let bytes_per_vec = dimension.div_ceil(8);
    let mut vec = vec![0.0f32; dimension];

    for (i, &byte) in data.iter().take(bytes_per_vec).enumerate() {
        for bit in 0..8 {
            let dim_idx = i * 8 + bit;
            if dim_idx < dimension {
                vec[dim_idx] = if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
            }
        }
    }
    vec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sq8_roundtrip() {
        let vector = vec![0.0, 0.5, 1.0, -0.5, 0.25];
        let params = compute_sq8_params(&vector);

        let mut quantized = Vec::new();
        quantize_sq8(&vector, params, &mut quantized);

        let dequantized = dequantize_sq8(&quantized, params);

        for (orig, deq) in vector.iter().zip(dequantized.iter()) {
            assert!((orig - deq).abs() < 0.01, "SQ8 roundtrip error too large");
        }
    }

    #[test]
    fn test_sq8_constant_vector() {
        let vector = vec![0.5, 0.5, 0.5, 0.5];
        let params = compute_sq8_params(&vector);
        assert_eq!(params.scale, 1.0); // Fallback for constant vectors
    }

    #[test]
    fn test_binary_roundtrip() {
        let vector = vec![1.0, -0.5, 0.0, 0.1, -1.0, 0.9, 0.0, 0.5, 1.0, -0.1];
        let dimension = vector.len();

        let mut packed = Vec::new();
        pack_binary(&vector, dimension, &mut packed);

        let unpacked = unpack_binary(&packed, dimension);

        // Binary only preserves sign (positive = 1.0, else = 0.0)
        let expected: Vec<f32> = vector.iter().map(|&v| if v > 0.0 { 1.0 } else { 0.0 }).collect();
        assert_eq!(unpacked, expected);
    }

    #[test]
    fn test_binary_packing_bits() {
        // 8 values that should pack into: 0b10101010 = 170
        let vector = vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let mut packed = Vec::new();
        pack_binary(&vector, 8, &mut packed);
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0], 0b10101010);
    }

    #[test]
    fn test_binary_dimension_not_multiple_of_8() {
        let vector = vec![1.0, 0.0, 1.0, 0.0, 1.0]; // 5 dimensions
        let mut packed = Vec::new();
        pack_binary(&vector, 5, &mut packed);
        assert_eq!(packed.len(), 1); // ceil(5/8) = 1 byte

        let unpacked = unpack_binary(&packed, 5);
        assert_eq!(unpacked.len(), 5);
        assert_eq!(unpacked, vec![1.0, 0.0, 1.0, 0.0, 1.0]);
    }
}
