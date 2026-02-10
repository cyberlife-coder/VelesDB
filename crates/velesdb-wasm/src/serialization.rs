//! Binary serialization for `VectorStore`.
//!
//! Provides efficient binary format for persistence.

use crate::{DistanceMetric, StorageMode, VectorStore};
use wasm_bindgen::JsValue;

/// Binary format header size.
pub const HEADER_SIZE: usize = 18;

/// Serializes a `VectorStore` to binary format.
///
/// Format:
/// - Magic: "VELS" (4 bytes)
/// - Version: 1 (1 byte)
/// - Dimension: u32 LE (4 bytes)
/// - Metric: u8 (1 byte)
/// - Count: u64 LE (8 bytes)
/// - Data: [id: u64, vector: f32 * dim] * count
pub fn export_to_bytes(store: &VectorStore) -> Vec<u8> {
    let count = store.ids.len();
    let vector_size = 8 + store.dimension * 4;
    let total_size = HEADER_SIZE + count * vector_size;
    let mut bytes = Vec::with_capacity(total_size);

    // Header: magic number "VELS"
    bytes.extend_from_slice(b"VELS");

    // Version
    bytes.push(1);

    // Dimension
    // Reason: WASM vector dimensions are always < 100K (model constraints), safely < u32::MAX
    #[allow(clippy::cast_possible_truncation)]
    let dim_u32 = store.dimension as u32;
    bytes.extend_from_slice(&dim_u32.to_le_bytes());

    // Metric
    let metric_byte = metric_to_byte(store.metric);
    bytes.push(metric_byte);

    // Vector count
    // Reason: WASM memory limits (4GB) prevent > u64::MAX vectors anyway
    let count_u64 = count as u64;
    bytes.extend_from_slice(&count_u64.to_le_bytes());

    // Data
    for (idx, &id) in store.ids.iter().enumerate() {
        bytes.extend_from_slice(&id.to_le_bytes());
        let start = idx * store.dimension;
        let data_slice = &store.data[start..start + store.dimension];
        let data_bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(data_slice.as_ptr().cast::<u8>(), store.dimension * 4)
        };
        bytes.extend_from_slice(data_bytes);
    }

    bytes
}

/// Deserializes a `VectorStore` from binary format.
///
/// Perf: Uses bulk copy for 500+ MB/s throughput.
pub fn import_from_bytes(bytes: &[u8]) -> Result<VectorStore, JsValue> {
    if bytes.len() < HEADER_SIZE {
        return Err(JsValue::from_str("Invalid data: too short"));
    }

    // Check magic
    if &bytes[0..4] != b"VELS" {
        return Err(JsValue::from_str("Invalid data: wrong magic number"));
    }

    // Check version
    let version = bytes[4];
    if version != 1 {
        return Err(JsValue::from_str(&format!(
            "Unsupported version: {version}"
        )));
    }

    // Read dimension
    let dimension = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;

    // Read metric
    let metric = byte_to_metric(bytes[9])?;

    // Read count
    // Reason: WASM memory limits prevent storing > usize::MAX vectors
    #[allow(clippy::cast_possible_truncation)]
    let count = u64::from_le_bytes([
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17],
    ]) as usize;

    // Validate data size
    let vector_size = 8 + dimension * 4;
    let expected_size = HEADER_SIZE + count * vector_size;
    if bytes.len() < expected_size {
        return Err(JsValue::from_str(&format!(
            "Invalid data: expected {expected_size} bytes, got {}",
            bytes.len()
        )));
    }

    // Perf: Pre-allocate contiguous buffers
    let mut ids = Vec::with_capacity(count);
    let total_floats = count * dimension;
    let mut data = vec![0.0_f32; total_floats];
    let data_bytes_len = dimension * 4;

    // Read all IDs first (cache-friendly sequential access)
    let mut offset = HEADER_SIZE;
    for _ in 0..count {
        let id = u64::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        ids.push(id);
        offset += 8 + data_bytes_len;
    }

    // Perf: Bulk copy all vector data
    // SAFETY: f32 and [u8; 4] have same size, WASM is little-endian
    let data_as_bytes: &mut [u8] = unsafe {
        core::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), total_floats * 4)
    };

    offset = HEADER_SIZE + 8; // Skip header + first ID
    for i in 0..count {
        let dest_start = i * data_bytes_len;
        let dest_end = dest_start + data_bytes_len;
        data_as_bytes[dest_start..dest_end]
            .copy_from_slice(&bytes[offset..offset + data_bytes_len]);
        offset += 8 + data_bytes_len;
    }

    Ok(VectorStore {
        ids,
        data,
        data_sq8: Vec::new(),
        data_binary: Vec::new(),
        sq8_mins: Vec::new(),
        sq8_scales: Vec::new(),
        payloads: vec![None; count],
        dimension,
        metric,
        storage_mode: StorageMode::Full,
    })
}

/// Converts a metric to its byte representation.
#[inline]
pub fn metric_to_byte(metric: DistanceMetric) -> u8 {
    match metric {
        DistanceMetric::Cosine => 0,
        DistanceMetric::Euclidean => 1,
        DistanceMetric::DotProduct => 2,
        DistanceMetric::Hamming => 3,
        DistanceMetric::Jaccard => 4,
    }
}

/// Converts a byte to its metric representation.
#[inline]
pub fn byte_to_metric(byte: u8) -> Result<DistanceMetric, JsValue> {
    match byte {
        0 => Ok(DistanceMetric::Cosine),
        1 => Ok(DistanceMetric::Euclidean),
        2 => Ok(DistanceMetric::DotProduct),
        3 => Ok(DistanceMetric::Hamming),
        4 => Ok(DistanceMetric::Jaccard),
        _ => Err(JsValue::from_str(&format!("Invalid metric byte: {byte}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metric_roundtrip() {
        let metrics = [
            DistanceMetric::Cosine,
            DistanceMetric::Euclidean,
            DistanceMetric::DotProduct,
            DistanceMetric::Hamming,
            DistanceMetric::Jaccard,
        ];
        for metric in metrics {
            let byte = metric_to_byte(metric);
            let result = byte_to_metric(byte).unwrap();
            assert_eq!(metric, result);
        }
    }
}
