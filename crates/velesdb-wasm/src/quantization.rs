//! Quantization helpers for VectorStore.
//!
//! This module contains SQ8 and Binary quantization logic extracted from lib.rs.

/// Quantize a vector to SQ8 format (8-bit per dimension).
///
/// Returns (quantized_bytes, min_value, scale_factor).
pub fn quantize_sq8(vector: &[f32]) -> (Vec<u8>, f32, f32) {
    let (min, max) = vector.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
        (min.min(v), max.max(v))
    });

    let scale = if (max - min).abs() < 1e-10 {
        1.0
    } else {
        255.0 / (max - min)
    };

    let quantized: Vec<u8> = vector
        .iter()
        .map(|&v| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let q = ((v - min) * scale).round().clamp(0.0, 255.0) as u8;
            q
        })
        .collect();

    (quantized, min, scale)
}

/// Dequantize SQ8 data back to f32.
pub fn dequantize_sq8(data: &[u8], min: f32, scale: f32) -> Vec<f32> {
    data.iter().map(|&q| (f32::from(q) / scale) + min).collect()
}

/// Pack a vector into binary format (1 bit per dimension).
pub fn pack_binary(vector: &[f32], dimension: usize) -> Vec<u8> {
    let bytes_needed = dimension.div_ceil(8);
    let mut bytes = Vec::with_capacity(bytes_needed);

    for byte_idx in 0..bytes_needed {
        let mut byte = 0u8;
        for bit in 0..8 {
            let dim_idx = byte_idx * 8 + bit;
            if dim_idx < dimension && vector[dim_idx] > 0.0 {
                byte |= 1 << bit;
            }
        }
        bytes.push(byte);
    }
    bytes
}

/// Unpack binary data back to f32 (0.0 or 1.0).
pub fn unpack_binary(data: &[u8], dimension: usize) -> Vec<f32> {
    let mut vec = vec![0.0f32; dimension];
    for (i, &byte) in data.iter().enumerate() {
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
        let original = vec![0.0, 0.5, 1.0, -0.5, -1.0];
        let (quantized, min, scale) = quantize_sq8(&original);
        let recovered = dequantize_sq8(&quantized, min, scale);

        for (o, r) in original.iter().zip(recovered.iter()) {
            assert!((o - r).abs() < 0.02, "SQ8 roundtrip error too large");
        }
    }

    #[test]
    fn test_binary_roundtrip() {
        let original = vec![1.0, -1.0, 0.5, -0.5, 0.0, 1.0, -1.0, 0.5];
        let packed = pack_binary(&original, original.len());
        let unpacked = unpack_binary(&packed, original.len());

        for (o, u) in original.iter().zip(unpacked.iter()) {
            let expected = if *o > 0.0 { 1.0 } else { 0.0 };
            assert_eq!(*u, expected);
        }
    }
}
