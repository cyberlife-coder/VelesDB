//! Binary serialization for `VectorStore`.
//!
//! Provides efficient binary format for persistence.

use crate::{DistanceMetric, StorageMode, VectorStore};
use wasm_bindgen::JsValue;

/// v1 header size (magic4 + version1 + dim4 + metric1 + count8). v1 stored only
/// id + f32 vectors (Full mode); kept for reading legacy data.
pub const HEADER_SIZE: usize = 18;

/// v2/v3 header size — v1 plus a storage-mode byte.
const HEADER_SIZE_V2: usize = 19;

/// Current format version written by [`export_to_bytes`].
const FORMAT_VERSION: u8 = 3;

#[path = "serialization_legacy.rs"]
mod legacy;

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

/// Serializes a `VectorStore` to the v3 binary format.
///
/// Layout (little-endian): `"VELS"`(4) | version=3(1) | dimension u32(4) |
/// metric u8(1) | storage-mode u8(1) | count u64(8), then `count` u64 IDs, then
/// one length-prefixed f32 blob holding the `count * dimension` vectors, then
/// `count` payload blobs (empty = None).
///
/// v3 is v2 without its four quantized blobs (`data_sq8`, `data_binary`,
/// `sq8_mins`, `sq8_scales`): every storage mode now keeps the f32 vectors, so
/// `data` is populated whatever the mode. The version bump makes an older
/// build reject a v3 blob ("Unsupported version: 3") rather than load a
/// quantized-mode store with empty code buffers. The sparse index is not
/// persisted (rebuilt on demand), as in v1/v2.
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
        version @ (2 | 3) => import_v2_or_v3(bytes, version),
        v => Err(JsValue::from_str(&format!("Unsupported version: {v}"))),
    }
}

/// Reads the v2 or v3 format. They share the header, the ids, the f32 `data`
/// blob and the payloads; v2 adds four quantized blobs between the last two,
/// from which [`legacy::reconstruct_vectors`] rebuilds the f32 vectors of a
/// quantized-mode store.
fn import_v2_or_v3(bytes: &[u8], version: u8) -> Result<VectorStore, JsValue> {
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
    let mut data = read_f32_blob(bytes, &mut offset)?;
    if version == 2 {
        let buffers = legacy::LegacyBuffers {
            data,
            data_sq8: read_blob(bytes, &mut offset)?.to_vec(),
            data_binary: read_blob(bytes, &mut offset)?.to_vec(),
            sq8_mins: read_f32_blob(bytes, &mut offset)?,
            sq8_scales: read_f32_blob(bytes, &mut offset)?,
        };
        data = legacy::reconstruct_vectors(storage_mode, count, dimension, buffers)
            .map_err(|e| JsValue::from_str(&e))?;
    }
    check_vector_count(data.len(), count, dimension).map_err(|e| JsValue::from_str(&e))?;
    let payloads = read_payloads(bytes, &mut offset, count)?;

    Ok(VectorStore {
        ids,
        data,
        payloads,
        dimension,
        metric,
        storage_mode,
        sparse_index: None,
    })
}

/// Checks that `data` holds exactly one `dimension`-float vector per id, so a
/// corrupt blob is rejected at load rather than read out of bounds at search.
fn check_vector_count(floats: usize, count: usize, dimension: usize) -> Result<(), String> {
    match count.checked_mul(dimension) {
        Some(expected) if expected == floats => Ok(()),
        Some(expected) => Err(format!(
            "Invalid data: {floats} vector floats for {count} ids of dimension {dimension} \
             (expected {expected})"
        )),
        None => Err("Invalid data: vector size arithmetic overflow".to_string()),
    }
}

/// Reads the legacy v1 format (id + f32 vectors only; Full mode, no payloads).
/// Checked byte/element sizes for a v1 blob body.
#[derive(Debug)]
pub(crate) struct V1Layout {
    /// Total expected blob length (header + all `count` vector records).
    pub expected_size: usize,
    /// Bytes of vector data per record (`dimension * 4`).
    pub data_bytes_len: usize,
    /// Total `f32` elements across all vectors (`count * dimension`).
    pub total_floats: usize,
}

/// Computes the [`V1Layout`] for `count` vectors of `dimension` floats, or an
/// error string on any size-arithmetic overflow.
///
/// Every product is `checked_*`. On `wasm32` `usize` is 32-bit and
/// `overflow-checks` is off in release, so an unchecked `count * dimension`
/// (or `count * vector_size`) could wrap, spoof `import_v1`'s length guard, and
/// admit an out-of-bounds read on a hostile or corrupt blob. Returns `String`
/// (not `JsValue`) so it is unit-testable off `wasm32`.
pub(crate) fn v1_layout(count: usize, dimension: usize) -> Result<V1Layout, String> {
    const OVERFLOW: &str = "Invalid data: vector size arithmetic overflow";
    let data_bytes_len = dimension.checked_mul(4).ok_or(OVERFLOW)?;
    let vector_size = data_bytes_len.checked_add(8).ok_or(OVERFLOW)?;
    let expected_size = count
        .checked_mul(vector_size)
        .and_then(|body| body.checked_add(HEADER_SIZE))
        .ok_or(OVERFLOW)?;
    let total_floats = count.checked_mul(dimension).ok_or(OVERFLOW)?;
    Ok(V1Layout {
        expected_size,
        data_bytes_len,
        total_floats,
    })
}

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

    // Validate data size with checked arithmetic (see `v1_layout`): on wasm32
    // `usize` is 32-bit and `overflow-checks` is off in release, so an unchecked
    // `count * dimension` could wrap, spoof the length guard, and admit an
    // out-of-bounds read on a hostile/corrupt blob.
    let layout = v1_layout(count, dimension).map_err(|e| JsValue::from_str(&e))?;
    if bytes.len() < layout.expected_size {
        return Err(JsValue::from_str(&format!(
            "Invalid data: expected {} bytes, got {}",
            layout.expected_size,
            bytes.len()
        )));
    }

    // Perf: Pre-allocate contiguous buffers
    let mut ids = Vec::with_capacity(count);
    let total_floats = layout.total_floats;
    let mut data = vec![0.0_f32; total_floats];
    let data_bytes_len = layout.data_bytes_len;

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
#[path = "serialization_tests.rs"]
mod tests;
