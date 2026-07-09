//! Binary serialization for `VectorStore`.
//!
//! Provides efficient binary format for persistence.

use crate::{DistanceMetric, StorageMode, VectorStore};
use wasm_bindgen::JsValue;

/// v1 header size (magic4 + version1 + dim4 + metric1 + count8). v1 stored only
/// id + f32 vectors (Full mode); kept for reading legacy data.
pub const HEADER_SIZE: usize = 18;

/// v2 header size — v1 plus a storage-mode byte.
const HEADER_SIZE_V2: usize = 19;

/// Current format version written by [`export_to_bytes`].
const FORMAT_VERSION: u8 = 2;

/// Maps a storage mode to its on-disk byte.
fn mode_to_byte(mode: StorageMode) -> u8 {
    match mode {
        StorageMode::Full => 0,
        StorageMode::SQ8 => 1,
        StorageMode::Binary => 2,
        StorageMode::ProductQuantization => 3,
        StorageMode::RaBitQ => 4,
    }
}

/// Maps an on-disk byte back to a storage mode.
fn byte_to_mode(byte: u8) -> Result<StorageMode, JsValue> {
    match byte {
        0 => Ok(StorageMode::Full),
        1 => Ok(StorageMode::SQ8),
        2 => Ok(StorageMode::Binary),
        3 => Ok(StorageMode::ProductQuantization),
        4 => Ok(StorageMode::RaBitQ),
        _ => Err(JsValue::from_str(&format!(
            "Invalid storage-mode byte: {byte}"
        ))),
    }
}

/// Appends a length-prefixed (u64 LE) byte blob.
fn write_blob(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Appends a length-prefixed blob of little-endian f32 values.
fn write_f32_blob(out: &mut Vec<u8>, values: &[f32]) {
    out.extend_from_slice(&((values.len() as u64) * 4).to_le_bytes());
    for &v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Appends the payloads section: each entry is a blob — empty for `None`,
/// otherwise the JSON bytes of the value.
fn write_payloads(out: &mut Vec<u8>, payloads: &[Option<serde_json::Value>]) {
    for payload in payloads {
        match payload {
            Some(v) => write_blob(out, &serde_json::to_vec(v).unwrap_or_default()),
            None => write_blob(out, &[]),
        }
    }
}

/// Serializes a `VectorStore` to the v2 binary format.
///
/// Layout (little-endian): `"VELS"`(4) | version=2(1) | dimension u32(4) |
/// metric u8(1) | storage-mode u8(1) | count u64(8), then `count` u64 IDs, then
/// five length-prefixed blobs (`data` f32, `data_sq8`, `data_binary`,
/// `sq8_mins` f32, `sq8_scales` f32), then `count` payload blobs (empty = None).
///
/// Unlike v1 (id + f32 only, Full mode), v2 preserves the storage mode, the
/// quantized buffers and the payloads, and never panics on a quantized store
/// (v1 indexed the empty `data` buffer in SQ8/Binary mode). The sparse index is
/// not persisted (rebuilt on demand), as in v1.
pub fn export_to_bytes(store: &VectorStore) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"VELS");
    bytes.push(FORMAT_VERSION);
    bytes.extend_from_slice(
        &u32::try_from(store.dimension)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    bytes.push(metric_to_byte(store.metric));
    bytes.push(mode_to_byte(store.storage_mode));
    bytes.extend_from_slice(&(store.ids.len() as u64).to_le_bytes());

    for &id in &store.ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    write_f32_blob(&mut bytes, &store.data);
    write_blob(&mut bytes, &store.data_sq8);
    write_blob(&mut bytes, &store.data_binary);
    write_f32_blob(&mut bytes, &store.sq8_mins);
    write_f32_blob(&mut bytes, &store.sq8_scales);
    write_payloads(&mut bytes, &store.payloads);
    bytes
}

/// Reads a u64 LE at `*offset`, advancing it. Bounds-checked.
fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, JsValue> {
    let end = offset
        .checked_add(8)
        .filter(|&e| e <= bytes.len())
        .ok_or_else(|| JsValue::from_str("Invalid data: truncated u64"))?;
    let arr: [u8; 8] = bytes[*offset..end]
        .try_into()
        .map_err(|_| JsValue::from_str("Invalid data: u64 slice"))?;
    *offset = end;
    Ok(u64::from_le_bytes(arr))
}

/// Reads a length-prefixed (u64 LE) byte blob, advancing `*offset`. Bounds-checked.
fn read_blob<'a>(bytes: &'a [u8], offset: &mut usize) -> Result<&'a [u8], JsValue> {
    let len = usize::try_from(read_u64(bytes, offset)?)
        .map_err(|_| JsValue::from_str("Invalid data: blob length too large"))?;
    let end = offset
        .checked_add(len)
        .filter(|&e| e <= bytes.len())
        .ok_or_else(|| JsValue::from_str("Invalid data: blob out of bounds"))?;
    let slice = &bytes[*offset..end];
    *offset = end;
    Ok(slice)
}

/// Reads a length-prefixed blob and decodes it as little-endian f32 values.
fn read_f32_blob(bytes: &[u8], offset: &mut usize) -> Result<Vec<f32>, JsValue> {
    let blob = read_blob(bytes, offset)?;
    if blob.len() % 4 != 0 {
        return Err(JsValue::from_str(
            "Invalid data: f32 blob not 4-byte aligned",
        ));
    }
    Ok(blob
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Reads `count` payload blobs (an empty blob decodes to `None`).
fn read_payloads(
    bytes: &[u8],
    offset: &mut usize,
    count: usize,
) -> Result<Vec<Option<serde_json::Value>>, JsValue> {
    let mut payloads = Vec::with_capacity(count);
    for _ in 0..count {
        let blob = read_blob(bytes, offset)?;
        if blob.is_empty() {
            payloads.push(None);
        } else {
            let value = serde_json::from_slice(blob)
                .map_err(|e| JsValue::from_str(&format!("Invalid payload JSON: {e}")))?;
            payloads.push(Some(value));
        }
    }
    Ok(payloads)
}

/// Deserializes a `VectorStore`, dispatching on the format version byte.
pub fn import_from_bytes(bytes: &[u8]) -> Result<VectorStore, JsValue> {
    if bytes.len() < 5 {
        return Err(JsValue::from_str("Invalid data: too short"));
    }
    if &bytes[0..4] != b"VELS" {
        return Err(JsValue::from_str("Invalid data: wrong magic number"));
    }
    match bytes[4] {
        1 => import_v1(bytes),
        2 => import_v2(bytes),
        v => Err(JsValue::from_str(&format!("Unsupported version: {v}"))),
    }
}

/// Reads the v2 format (storage mode, quantized buffers and payloads preserved).
fn import_v2(bytes: &[u8]) -> Result<VectorStore, JsValue> {
    if bytes.len() < HEADER_SIZE_V2 {
        return Err(JsValue::from_str("Invalid data: too short"));
    }
    let dimension = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
    let metric = byte_to_metric(bytes[9])?;
    let storage_mode = byte_to_mode(bytes[10])?;
    let count = usize::try_from(u64::from_le_bytes([
        bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18],
    ]))
    .map_err(|_| JsValue::from_str("Invalid data: count too large"))?;
    let mut offset = HEADER_SIZE_V2;
    // DoS guard: each ID needs 8 bytes, so `count` cannot exceed the remaining
    // input — reject before allocating.
    if count > bytes.len().saturating_sub(offset) / 8 {
        return Err(JsValue::from_str("Invalid data: count exceeds input size"));
    }

    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(read_u64(bytes, &mut offset)?);
    }
    let data = read_f32_blob(bytes, &mut offset)?;
    let data_sq8 = read_blob(bytes, &mut offset)?.to_vec();
    let data_binary = read_blob(bytes, &mut offset)?.to_vec();
    let sq8_mins = read_f32_blob(bytes, &mut offset)?;
    let sq8_scales = read_f32_blob(bytes, &mut offset)?;
    let payloads = read_payloads(bytes, &mut offset, count)?;

    Ok(VectorStore {
        ids,
        data,
        data_sq8,
        data_binary,
        sq8_mins,
        sq8_scales,
        payloads,
        dimension,
        metric,
        storage_mode,
        sparse_index: None,
    })
}

/// Reads the legacy v1 format (id + f32 vectors only; Full mode, no payloads).
fn import_v1(bytes: &[u8]) -> Result<VectorStore, JsValue> {
    if bytes.len() < HEADER_SIZE {
        return Err(JsValue::from_str("Invalid data: too short"));
    }

    // Read dimension
    let dimension = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;

    // Read metric
    let metric = byte_to_metric(bytes[9])?;

    // Read count
    // Reason: WASM memory limits prevent storing > usize::MAX vectors
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
    // SAFETY: Reinterprets the `Vec<f32>` allocation as mutable bytes for bulk deserialization.
    // - `data` is a valid, uniquely-owned Vec<f32> freshly allocated on the heap in this function;
    //   it cannot alias `bytes`, which is a shared reference provided by the caller.
    // - The length passed is `total_floats * 4`, which is the exact byte size of the `data`
    //   allocation (count * dimension * sizeof(f32) == count * dimension * 4).
    // - WASM is guaranteed little-endian, matching the on-wire format written by `export_to_bytes`.
    // Reason: avoids per-element f32::from_le_bytes() for significantly higher throughput.
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
        sparse_index: None,
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
        // Reason: DistanceMetric is #[non_exhaustive] — future variants map to 255 (unknown).
        _ => 255,
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

    #[test]
    fn test_v2_roundtrip_full_preserves_payloads() {
        let store = VectorStore {
            ids: vec![1, 2],
            data: vec![1.0, 2.0, 3.0, 4.0],
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: vec![Some(serde_json::json!({"k": "v"})), None],
            dimension: 2,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
            sparse_index: None,
        };
        let restored = import_from_bytes(&export_to_bytes(&store)).unwrap();
        assert_eq!(restored.ids, vec![1, 2]);
        assert_eq!(restored.data, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(
            mode_to_byte(restored.storage_mode),
            mode_to_byte(StorageMode::Full)
        );
        assert_eq!(restored.payloads[0], Some(serde_json::json!({"k": "v"})));
        assert_eq!(restored.payloads[1], None);
    }

    #[test]
    fn test_v2_roundtrip_sq8_preserves_mode_buffers_payload() {
        // v1 dropped the SQ8 buffers/mode (reloaded as Full) and all payloads.
        let store = VectorStore {
            ids: vec![10],
            data: Vec::new(),
            data_sq8: vec![200, 100],
            data_binary: Vec::new(),
            sq8_mins: vec![0.5],
            sq8_scales: vec![0.01],
            payloads: vec![Some(serde_json::json!({"x": 1}))],
            dimension: 2,
            metric: DistanceMetric::Euclidean,
            storage_mode: StorageMode::SQ8,
            sparse_index: None,
        };
        let restored = import_from_bytes(&export_to_bytes(&store)).unwrap();
        assert_eq!(
            mode_to_byte(restored.storage_mode),
            mode_to_byte(StorageMode::SQ8)
        );
        assert_eq!(restored.data_sq8, vec![200, 100]);
        assert_eq!(restored.sq8_mins, vec![0.5]);
        assert_eq!(restored.sq8_scales, vec![0.01]);
        assert_eq!(restored.payloads[0], Some(serde_json::json!({"x": 1})));
    }

    #[test]
    fn test_v2_roundtrip_binary_does_not_panic() {
        // v1 panicked here by indexing the empty `data` buffer in Binary mode.
        let store = VectorStore {
            ids: vec![7],
            data: Vec::new(),
            data_sq8: Vec::new(),
            data_binary: vec![0b1010_1010],
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: vec![None],
            dimension: 8,
            metric: DistanceMetric::Hamming,
            storage_mode: StorageMode::Binary,
            sparse_index: None,
        };
        let restored = import_from_bytes(&export_to_bytes(&store)).unwrap();
        assert_eq!(
            mode_to_byte(restored.storage_mode),
            mode_to_byte(StorageMode::Binary)
        );
        assert_eq!(restored.data_binary, vec![0b1010_1010]);
    }
}
