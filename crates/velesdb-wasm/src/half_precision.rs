//! WASM bindings for half-precision (f16/bf16) vector conversion.
//!
//! Reduces WASM memory usage by 50% for embedding storage.
//! Converts between f32 and f16/bf16 byte representations.

use wasm_bindgen::prelude::*;

use velesdb_core::half_precision::{VectorData, VectorPrecision};

/// Convert an f32 vector to f16 bytes (2 bytes per element, 50% memory savings).
///
/// Returns a `Uint8Array` of raw f16 bytes.
#[wasm_bindgen]
pub fn f32_to_f16(vector: &[f32]) -> Vec<u8> {
    let data = VectorData::from_f32_slice(vector, VectorPrecision::F16);
    match data {
        VectorData::F16(v) => {
            let mut bytes = Vec::with_capacity(v.len() * 2);
            for val in &v {
                bytes.extend_from_slice(&val.to_le_bytes());
            }
            bytes
        }
        // Reason: from_f32_slice with F16 always returns VectorData::F16
        _ => unreachable!(),
    }
}

/// Convert f16 bytes back to an f32 vector.
///
/// Input must have even length (2 bytes per element).
#[wasm_bindgen]
pub fn f16_to_f32(bytes: &[u8]) -> Result<Vec<f32>, JsValue> {
    if bytes.len() % 2 != 0 {
        return Err(JsValue::from_str("f16 bytes must have even length"));
    }
    let result: Vec<f32> = bytes
        .chunks_exact(2)
        .map(|chunk| {
            let bits = u16::from_le_bytes([chunk[0], chunk[1]]);
            half::f16::from_bits(bits).to_f32()
        })
        .collect();
    Ok(result)
}

/// Convert an f32 vector to bf16 bytes (2 bytes per element, ML-optimized).
///
/// bf16 preserves the same exponent range as f32, better for ML workloads.
#[wasm_bindgen]
pub fn f32_to_bf16(vector: &[f32]) -> Vec<u8> {
    let data = VectorData::from_f32_slice(vector, VectorPrecision::BF16);
    match data {
        VectorData::BF16(v) => {
            let mut bytes = Vec::with_capacity(v.len() * 2);
            for val in &v {
                bytes.extend_from_slice(&val.to_le_bytes());
            }
            bytes
        }
        // Reason: from_f32_slice with BF16 always returns VectorData::BF16
        _ => unreachable!(),
    }
}

/// Convert bf16 bytes back to an f32 vector.
///
/// Input must have even length (2 bytes per element).
#[wasm_bindgen]
pub fn bf16_to_f32(bytes: &[u8]) -> Result<Vec<f32>, JsValue> {
    if bytes.len() % 2 != 0 {
        return Err(JsValue::from_str("bf16 bytes must have even length"));
    }
    let result: Vec<f32> = bytes
        .chunks_exact(2)
        .map(|chunk| {
            let bits = u16::from_le_bytes([chunk[0], chunk[1]]);
            half::bf16::from_bits(bits).to_f32()
        })
        .collect();
    Ok(result)
}

/// Returns the memory size in bytes for a vector of given dimension and precision.
///
/// `precision` is one of: `"f32"`, `"f16"`, `"bf16"`.
#[wasm_bindgen]
pub fn vector_memory_size(dimension: usize, precision: &str) -> Result<usize, JsValue> {
    let p = match precision.to_lowercase().as_str() {
        "f32" => VectorPrecision::F32,
        "f16" => VectorPrecision::F16,
        "bf16" => VectorPrecision::BF16,
        _ => {
            return Err(JsValue::from_str(
                "Unknown precision. Valid: f32, f16, bf16",
            ))
        }
    };
    Ok(p.memory_size(dimension))
}

#[cfg(test)]
mod tests {
    use velesdb_core::half_precision::{VectorData, VectorPrecision};

    #[test]
    fn test_f32_f16_roundtrip() {
        let original = vec![1.0_f32, 2.0, 3.0, -1.5, 0.0];
        let data = VectorData::from_f32_slice(&original, VectorPrecision::F16);
        assert_eq!(data.len(), original.len());
        assert_eq!(data.memory_size(), original.len() * 2); // 50% savings

        let recovered = data.to_f32_vec();
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 0.01, "f16 roundtrip: {a} vs {b}");
        }
    }

    #[test]
    fn test_f32_bf16_roundtrip() {
        let original = vec![1.0_f32, 2.0, 3.0, -1.5, 0.0];
        let data = VectorData::from_f32_slice(&original, VectorPrecision::BF16);
        assert_eq!(data.len(), original.len());

        let recovered = data.to_f32_vec();
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 0.02, "bf16 roundtrip: {a} vs {b}");
        }
    }

    #[test]
    fn test_f16_memory_savings() {
        let dim = 768; // BERT dimension
        let f32_size = VectorPrecision::F32.memory_size(dim);
        let f16_size = VectorPrecision::F16.memory_size(dim);
        let bf16_size = VectorPrecision::BF16.memory_size(dim);
        assert_eq!(f32_size, dim * 4);
        assert_eq!(f16_size, dim * 2);
        assert_eq!(bf16_size, dim * 2);
        assert_eq!(f16_size * 2, f32_size); // Exactly 50% savings
    }

    #[test]
    fn test_empty_vector() {
        let data = VectorData::from_f32_slice(&[], VectorPrecision::F16);
        assert!(data.is_empty());
        assert_eq!(data.memory_size(), 0);
        assert!(data.to_f32_vec().is_empty());
    }

    #[test]
    fn test_precision_enum() {
        assert_eq!(VectorPrecision::F32.bytes_per_element(), 4);
        assert_eq!(VectorPrecision::F16.bytes_per_element(), 2);
        assert_eq!(VectorPrecision::BF16.bytes_per_element(), 2);
    }

    #[test]
    fn test_convert_precision() {
        let f32_data = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
        let f16_data = f32_data.convert(VectorPrecision::F16);
        assert!(matches!(f16_data.precision(), VectorPrecision::F16));

        let back = f16_data.convert(VectorPrecision::F32);
        let result = back.to_f32_vec();
        assert!((result[0] - 1.0).abs() < 0.01);
    }
}
