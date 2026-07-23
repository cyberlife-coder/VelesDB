//! Quantization helpers for vector storage.
//!
//! Provides SQ8 (8-bit scalar quantization) and Binary (1-bit) quantization
//! for memory-efficient vector storage.
//!
//! # Status (#1543)
//!
//! This module is **not currently wired into the crate** — there is no
//! `mod quantization;` declaration in `lib.rs`, so `cargo build`/`test`
//! never compiles this file and it has no effect on the shipped WASM
//! binary. The production encode/decode paths live in
//! `store_insert::encode_sq8`/`encode_binary`, `vector_ops::ScratchBuffer`
//! and `store_get::decode_sq8` — those are what actually run in the
//! browser and are what the #1543 core-parity fix targets. This file is
//! kept aligned with the same core-parity fixes (same constants, same
//! degenerate-range fallback) so it does not silently rot as a misleading
//! reference if it is ever wired in or used as a refactor starting point.

/// SQ8 quantization parameters for a single vector.
#[derive(Debug, Clone, Copy)]
pub struct Sq8Params {
    pub min: f32,
    pub scale: f32,
}

/// Computes SQ8 quantization parameters (min, scale) for a vector.
///
/// Mirrors `velesdb_core::QuantizedVector::from_f32` exactly (#1543): same
/// min/max fold seeds (`f32::INFINITY`/`f32::NEG_INFINITY`) and the same
/// degenerate-range threshold (`range < f32::EPSILON`, not the old ad hoc
/// `1e-10` guess). `scale == 0.0` is used as an in-band sentinel for
/// "degenerate range" — see `quantize_sq8`/`dequantize_sq8`.
#[must_use]
pub fn compute_sq8_params(vector: &[f32]) -> Sq8Params {
    let (min, max) = vector
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &v| (min.min(v), max.max(v)));

    let range = max - min;
    let scale = if range < f32::EPSILON { 0.0 } else { 255.0 / range };

    Sq8Params { min, scale }
}

/// Quantizes a vector to SQ8 format using pre-computed parameters.
///
/// Each f32 value is mapped to u8 range [0, 255]. A degenerate range
/// (`params.scale == 0.0`, see `compute_sq8_params`) fills every dimension
/// with byte 128, matching core's `QuantizedVector::from_f32` fallback for
/// constant/near-constant vectors exactly.
pub fn quantize_sq8(vector: &[f32], params: Sq8Params, output: &mut Vec<u8>) {
    #[allow(clippy::float_cmp)]
    if params.scale == 0.0 {
        let new_len = output.len() + vector.len();
        output.resize(new_len, 128u8);
        return;
    }
    for &v in vector {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let quantized = ((v - params.min) * params.scale).round().clamp(0.0, 255.0) as u8;
        output.push(quantized);
    }
}

/// Dequantizes SQ8 data back to f32.
///
/// Mirrors core's `QuantizedVector::to_f32` fallback: a degenerate range
/// (`params.scale == 0.0`) decodes every dimension to `min` instead of
/// dividing by zero.
#[must_use]
pub fn dequantize_sq8(data: &[u8], params: Sq8Params) -> Vec<f32> {
    #[allow(clippy::float_cmp)]
    if params.scale == 0.0 {
        return vec![params.min; data.len()];
    }
    data.iter()
        .map(|&q| (f32::from(q) / params.scale) + params.min)
        .collect()
}

/// Packs a vector into binary format (1 bit per dimension).
///
/// Values `>= 0.0` become 1, values `< 0.0` become 0 — matching the core
/// `BinaryQuantizedVector` sign convention exactly.
/// Bits are packed 8 per byte, LSB first.
pub fn pack_binary(vector: &[f32], dimension: usize, output: &mut Vec<u8>) {
    let bytes_needed = dimension.div_ceil(8);
    for byte_idx in 0..bytes_needed {
        let mut byte = 0u8;
        for bit in 0..8 {
            let dim_idx = byte_idx * 8 + bit;
            if dim_idx < dimension && vector[dim_idx] >= 0.0 {
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
        // scale == 0.0 is the degenerate-range sentinel (matches core's
        // QuantizedVector::from_f32 128-fill fallback exactly).
        assert_eq!(params.scale, 0.0);

        let mut quantized = Vec::new();
        quantize_sq8(&vector, params, &mut quantized);
        assert_eq!(quantized, vec![128u8; 4]);

        let dequantized = dequantize_sq8(&quantized, params);
        assert_eq!(dequantized, vec![0.5f32; 4]);
    }

    #[test]
    fn test_binary_roundtrip() {
        let vector = vec![1.0, -0.5, 0.0, 0.1, -1.0, 0.9, -0.2, 0.5, 1.0, -0.1];
        let dimension = vector.len();

        let mut packed = Vec::new();
        pack_binary(&vector, dimension, &mut packed);

        let unpacked = unpack_binary(&packed, dimension);

        // Binary only preserves sign: matches core convention (>= 0.0 -> 1.0, else 0.0).
        let expected: Vec<f32> = vector.iter().map(|&v| if v >= 0.0 { 1.0 } else { 0.0 }).collect();
        assert_eq!(unpacked, expected);
    }

    #[test]
    fn test_binary_packing_bits() {
        // Negative values -> 0, non-negative -> 1, packed LSB first into 0b10101010 = 170.
        let vector = vec![-1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0];
        let mut packed = Vec::new();
        pack_binary(&vector, 8, &mut packed);
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0], 0b10101010);
    }

    #[test]
    fn test_binary_zero_packs_as_one() {
        // Core convention: 0.0 is non-negative, so it must pack as bit 1.
        let vector = vec![0.0, -0.1, 0.0];
        let mut packed = Vec::new();
        pack_binary(&vector, 3, &mut packed);
        let unpacked = unpack_binary(&packed, 3);
        assert_eq!(unpacked, vec![1.0, 0.0, 1.0]);
    }

    #[test]
    fn test_binary_dimension_not_multiple_of_8() {
        let vector = vec![1.0, -1.0, 1.0, -1.0, 1.0]; // 5 dimensions
        let mut packed = Vec::new();
        pack_binary(&vector, 5, &mut packed);
        assert_eq!(packed.len(), 1); // ceil(5/8) = 1 byte

        let unpacked = unpack_binary(&packed, 5);
        assert_eq!(unpacked.len(), 5);
        assert_eq!(unpacked, vec![1.0, 0.0, 1.0, 0.0, 1.0]);
    }
}
