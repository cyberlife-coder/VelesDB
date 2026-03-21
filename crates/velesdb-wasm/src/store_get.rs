//! Get operations for `VectorStore`.

use crate::{StorageMode, VectorStore};
use wasm_bindgen::JsValue;

/// Gets a vector by index, reconstructing from storage mode.
pub fn get_vector_at_index(store: &VectorStore, idx: usize) -> Vec<f32> {
    match store.storage_mode {
        StorageMode::Full => {
            let start = idx * store.dimension;
            store.data[start..start + store.dimension].to_vec()
        }
        // ProductQuantization uses SQ8 path as fallback in WASM context
        // (PQ codebooks are not available in the lightweight WASM store)
        StorageMode::SQ8 | StorageMode::ProductQuantization => decode_sq8(
            &store.data_sq8,
            store.sq8_mins[idx],
            store.sq8_scales[idx],
            idx,
            store.dimension,
        ),
        StorageMode::Binary => decode_binary(&store.data_binary, idx, store.dimension),
    }
}

/// Decodes a single SQ8-quantized vector back to f32.
fn decode_sq8(data_sq8: &[u8], min: f32, scale: f32, idx: usize, dimension: usize) -> Vec<f32> {
    let start = idx * dimension;
    data_sq8[start..start + dimension]
        .iter()
        .map(|&q| (f32::from(q) / scale) + min)
        .collect()
}

/// Decodes a single binary-quantized vector back to f32 (0.0 / 1.0).
fn decode_binary(data_binary: &[u8], idx: usize, dimension: usize) -> Vec<f32> {
    let bytes_per_vec = dimension.div_ceil(8);
    let start = idx * bytes_per_vec;
    let mut vec = vec![0.0f32; dimension];
    for (i, &byte) in data_binary[start..start + bytes_per_vec].iter().enumerate() {
        for bit in 0..8 {
            let dim_idx = i * 8 + bit;
            if dim_idx < dimension {
                vec[dim_idx] = if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
            }
        }
    }
    vec
}

/// Gets a vector by ID, returning JSON result.
pub fn get_by_id(store: &VectorStore, id: u64) -> Result<JsValue, JsValue> {
    let idx = match store.ids.iter().position(|&x| x == id) {
        Some(i) => i,
        None => return Ok(JsValue::NULL),
    };

    let vector = get_vector_at_index(store, idx);
    let result = serde_json::json!({
        "id": id,
        "vector": vector,
        "payload": store.payloads[idx]
    });

    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}
