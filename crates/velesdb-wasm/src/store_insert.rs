//! Insert operations for `VectorStore`.

use crate::{store_search, VectorStore};

#[cfg(test)]
#[path = "store_insert_tests.rs"]
mod tests;

/// Inserts a vector into the store.
///
/// The f32 vector is stored as-is whatever the storage mode, as in
/// `velesdb-core`: the mode never makes search lossy (see `vector_ops`).
pub fn insert_vector(store: &mut VectorStore, id: u64, vector: &[f32]) {
    insert_with_payload(store, id, vector, None);
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
/// upsert-dedup behaves identically to single insert. No payloads — use
/// `insert_batch` for those.
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

/// Inserts a vector with payload, replacing any existing vector with the same ID.
pub fn insert_with_payload(
    store: &mut VectorStore,
    id: u64,
    vector: &[f32],
    payload: Option<serde_json::Value>,
) {
    if let Some(idx) = store.ids.iter().position(|&x| x == id) {
        remove_at_index(store, idx);
    }

    store.ids.push(id);
    store.payloads.push(payload);
    store.data.extend_from_slice(vector);
}

/// Removes a vector at the given index.
///
/// The parallel arrays (`ids`, `payloads`) and the chunked `data` buffer are
/// updated with matching `swap_remove` semantics so that they stay in sync:
/// mixing `swap_remove` on one and `drain` on the other desynchronises ids
/// from the vector at `idx`. Order is not preserved by removal anyway, so
/// swap-remove is both cheaper and correct.
pub fn remove_at_index(store: &mut VectorStore, idx: usize) {
    store.ids.swap_remove(idx);
    store.payloads.swap_remove(idx);
    swap_remove_chunk(&mut store.data, idx, store.dimension);
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
