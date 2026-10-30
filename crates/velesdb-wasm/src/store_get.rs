//! Get operations for `VectorStore`.

use crate::VectorStore;
use wasm_bindgen::JsValue;

/// Gets the vector stored at `idx` — the inserted f32 values, in every
/// storage mode (as `velesdb-core` returns them).
pub fn get_vector_at_index(store: &VectorStore, idx: usize) -> Vec<f32> {
    let start = idx * store.dimension;
    store.data[start..start + store.dimension].to_vec()
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

    crate::js_value::to_js_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}
