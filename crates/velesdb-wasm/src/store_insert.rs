//! Insert operations for `VectorStore`.

use crate::{StorageMode, VectorStore};

/// Inserts a vector into the store based on storage mode.
pub fn insert_vector(store: &mut VectorStore, id: u64, vector: &[f32]) {
    // Remove existing vector with same ID if present
    if let Some(idx) = store.ids.iter().position(|&x| x == id) {
        remove_at_index(store, idx);
    }

    store.ids.push(id);
    store.payloads.push(None);

    match store.storage_mode {
        StorageMode::Full => {
            store.data.extend_from_slice(vector);
        }
        StorageMode::SQ8 => {
            let (min, max) = vector.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
                (min.min(v), max.max(v))
            });
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
        StorageMode::Binary => {
            let bytes_needed = store.dimension.div_ceil(8);
            for byte_idx in 0..bytes_needed {
                let mut byte = 0u8;
                for bit in 0..8 {
                    let dim_idx = byte_idx * 8 + bit;
                    if dim_idx < store.dimension && vector[dim_idx] > 0.0 {
                        byte |= 1 << bit;
                    }
                }
                store.data_binary.push(byte);
            }
        }
    }
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

    match store.storage_mode {
        StorageMode::Full => {
            store.data.extend_from_slice(vector);
        }
        StorageMode::SQ8 => {
            let (min, max) = vector.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
                (min.min(v), max.max(v))
            });
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
        StorageMode::Binary => {
            let bytes_needed = store.dimension.div_ceil(8);
            for byte_idx in 0..bytes_needed {
                let mut byte = 0u8;
                for bit in 0..8 {
                    let dim_idx = byte_idx * 8 + bit;
                    if dim_idx < store.dimension && vector[dim_idx] > 0.0 {
                        byte |= 1 << bit;
                    }
                }
                store.data_binary.push(byte);
            }
        }
    }
}

/// Removes a vector at the given index.
pub fn remove_at_index(store: &mut VectorStore, idx: usize) {
    store.ids.swap_remove(idx);
    store.payloads.swap_remove(idx);

    match store.storage_mode {
        StorageMode::Full => {
            let start = idx * store.dimension;
            let end = start + store.dimension;
            store.data.drain(start..end);
        }
        StorageMode::SQ8 => {
            store.sq8_mins.swap_remove(idx);
            store.sq8_scales.swap_remove(idx);
            let start = idx * store.dimension;
            let end = start + store.dimension;
            store.data_sq8.drain(start..end);
        }
        StorageMode::Binary => {
            let bytes_per = store.dimension.div_ceil(8);
            let start = idx * bytes_per;
            let end = start + bytes_per;
            store.data_binary.drain(start..end);
        }
    }
}
