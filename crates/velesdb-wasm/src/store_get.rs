//! Get operations for `VectorStore`.

use crate::StoreState;
use velesdb_core::api_types::serde_id::id_to_json;
use wasm_bindgen::JsValue;

/// Gets the vector stored at `idx` — the inserted f32 values, in every
/// storage mode (as `velesdb-core` returns them).
pub fn get_vector_at_index(store: &StoreState, idx: usize) -> Vec<f32> {
    let start = idx * store.dimension;
    store.data[start..start + store.dimension].to_vec()
}

/// Gets a vector by ID: `{id, vector, payload}`, or `null` when absent.
pub fn get_by_id(store: &StoreState, id: u64) -> Result<JsValue, JsValue> {
    match point_json(store, id) {
        Some(point) => {
            crate::js_value::to_js_value(&point).map_err(|e| JsValue::from_str(&e.to_string()))
        }
        None => Ok(JsValue::NULL),
    }
}

/// The point stored under `id` as `{id, vector, payload}`, the id its exact
/// decimal string as REST sends it; `None` when the store has no such point.
pub(crate) fn point_json(store: &StoreState, id: u64) -> Option<serde_json::Value> {
    let idx = store.ids.iter().position(|&x| x == id)?;
    Some(serde_json::json!({
        "id": id_to_json(id),
        "vector": get_vector_at_index(store, idx),
        "payload": store.payloads[idx]
    }))
}
