//! Insert operations for `VectorStore`.

use crate::{store_search, StorageMode, VectorStore};

#[cfg(test)]
#[path = "store_insert_tests.rs"]
mod tests;

/// Encodes a vector into the store's buffers based on storage mode.
///
/// This is the single encoding path for all insert operations.
///
/// `ProductQuantization` and `RaBitQ` deliberately share the SQ8 encode path:
/// the browser engine has no PQ codebook training, so those modes fall back to
/// SQ8 quantization. The fallback is surfaced once via `console.warn` at store
/// creation (see `parsing::parse_storage_mode_inner`), so it is not silent.
fn encode_vector(store: &mut VectorStore, vector: &[f32]) {
    match store.storage_mode {
        StorageMode::Full => {
            store.data.extend_from_slice(vector);
        }
        StorageMode::SQ8 | StorageMode::ProductQuantization | StorageMode::RaBitQ => {
            encode_sq8(store, vector);
        }
        StorageMode::Binary => {
            encode_binary(store, vector);
        }
    }
}

/// SQ8 scalar quantization: maps f32 range to 0-255.
fn encode_sq8(store: &mut VectorStore, vector: &[f32]) {
    let (min, max) = vector
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let scale = if (max - min).abs() < 1e-10 {
        1.0
    } else {
        255.0 / (max - min)
    };

    store.sq8_mins.push(min);
    store.sq8_scales.push(scale);

    for &v in vector {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let quantized = ((v - min) * scale).round().clamp(0.0, 255.0) as u8;
        store.data_sq8.push(quantized);
    }
}

/// Binary quantization: packs each dimension into a single bit.
///
/// Values `>= 0.0` become 1, values `< 0.0` become 0 — matching the core
/// `BinaryQuantizedVector` sign convention exactly.
fn encode_binary(store: &mut VectorStore, vector: &[f32]) {
    let bytes_needed = store.dimension.div_ceil(8);
    for byte_idx in 0..bytes_needed {
        let mut byte = 0u8;
        for bit in 0..8 {
            let dim_idx = byte_idx * 8 + bit;
            if dim_idx < store.dimension && vector[dim_idx] >= 0.0 {
                byte |= 1 << bit;
            }
        }
        store.data_binary.push(byte);
    }
}

/// Inserts a vector into the store based on storage mode.
pub fn insert_vector(store: &mut VectorStore, id: u64, vector: &[f32]) {
    // Remove existing vector with same ID if present
    if let Some(idx) = store.ids.iter().position(|&x| x == id) {
        remove_at_index(store, idx);
    }

    store.ids.push(id);
    store.payloads.push(None);
    encode_vector(store, vector);
}

/// Validates a flat raw batch against the store geometry.
///
/// Checks that `dimension` matches the store, then delegates the
/// `checked_mul` overflow guard and `ids.len() * dimension` length check to
/// [`store_search::validate_multi_vector_len`].
fn validate_raw_batch(
    store: &VectorStore,
    ids: &[u64],
    vectors: &[f32],
    dimension: usize,
) -> Result<(), String> {
    if dimension != store.dimension {
        return Err(format!(
            "dimension mismatch: expected {}, got {dimension}",
            store.dimension
        ));
    }
    store_search::validate_multi_vector_len(vectors.len(), ids.len(), dimension)?;
    Ok(())
}

/// Bulk insert from a flat row-major `vectors` buffer plus parallel `ids`.
///
/// Mirrors the VRB1 raw-bulk wire contract (flat `Float32Array` + `u64`
/// ids + explicit `dimension`). Reuses [`insert_vector`] per row so the
/// upsert-dedup and per-mode encoding (Full/SQ8/Binary) behave identically
/// to single insert. No payloads — use `insert_batch` for those.
pub fn insert_batch_raw(
    store: &mut VectorStore,
    ids: &[u64],
    vectors: &[f32],
    dimension: usize,
) -> Result<(), String> {
    validate_raw_batch(store, ids, vectors, dimension)?;
    store.ids.reserve(ids.len());
    store.data.reserve(vectors.len());
    for (i, &id) in ids.iter().enumerate() {
        let v = &vectors[i * dimension..(i + 1) * dimension];
        insert_vector(store, id, v);
    }
    Ok(())
}

/// Inserts a vector with payload.
pub fn insert_with_payload(
    store: &mut VectorStore,
    id: u64,
    vector: &[f32],
    payload: Option<serde_json::Value>,
) {
    // Remove existing if present
    if let Some(idx) = store.ids.iter().position(|&x| x == id) {
        remove_at_index(store, idx);
    }

    store.ids.push(id);
    store.payloads.push(payload);
    encode_vector(store, vector);
}

/// Removes a vector at the given index.
///
/// All parallel arrays (`ids`, `payloads`, `sq8_mins`, `sq8_scales`, and
/// the per-mode data buffer) are updated with matching `swap_remove`
/// semantics so that they stay in sync. Previously the id/payload/min/scale
/// arrays used `swap_remove` (O(1), swaps the last element into `idx`)
/// while `data` / `data_sq8` / `data_binary` used `drain` (O(n), shifts
/// everything left). When removing a non-last index, this desynchronised
/// ids from the vector bytes at `idx`. The fix is to use swap-remove on
/// the chunked buffers too — order is not preserved by removal anyway, so
/// this is both cheaper and correct.
pub fn remove_at_index(store: &mut VectorStore, idx: usize) {
    store.ids.swap_remove(idx);
    store.payloads.swap_remove(idx);

    match store.storage_mode {
        StorageMode::Full => {
            swap_remove_chunk(&mut store.data, idx, store.dimension);
        }
        // ProductQuantization/RaBitQ use SQ8 path as fallback in WASM context
        StorageMode::SQ8 | StorageMode::ProductQuantization | StorageMode::RaBitQ => {
            store.sq8_mins.swap_remove(idx);
            store.sq8_scales.swap_remove(idx);
            swap_remove_chunk(&mut store.data_sq8, idx, store.dimension);
        }
        StorageMode::Binary => {
            let bytes_per = store.dimension.div_ceil(8);
            swap_remove_chunk(&mut store.data_binary, idx, bytes_per);
        }
    }
}

/// Swap-removes a contiguous chunk of `chunk_size` elements starting at
/// `idx * chunk_size`, mirroring `Vec::swap_remove` for the parallel id
/// / payload arrays.
///
/// Edge cases:
/// - `chunk_size == 0` (metadata-only collection): no-op.
/// - `idx` points to the last chunk: truncate only.
/// - Any other index: swap the chunk at `idx` with the last chunk, then
///   truncate.
///
/// # Panics
/// Debug-asserts that `buf.len() >= (idx + 1) * chunk_size`. In release
/// builds the caller guarantees this (the id array was checked before).
fn swap_remove_chunk<T: Copy>(buf: &mut Vec<T>, idx: usize, chunk_size: usize) {
    if chunk_size == 0 {
        return;
    }
    debug_assert!(buf.len() >= (idx + 1) * chunk_size);
    let last_chunk_start = buf.len() - chunk_size;
    let target_start = idx * chunk_size;
    if target_start != last_chunk_start {
        for offset in 0..chunk_size {
            buf.swap(target_start + offset, last_chunk_start + offset);
        }
    }
    buf.truncate(last_chunk_start);
}
