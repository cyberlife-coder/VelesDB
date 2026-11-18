//! `AgentMemory` WASM bindings (EPIC-016 US-003)
//!
//! Provides semantic memory for AI agents in the browser.

use serde::{Deserialize, Serialize};
use velesdb_core::api_types::serde_id;
use wasm_bindgen::prelude::*;

#[cfg(doc)]
use crate::VectorStore;
use crate::{store_insert, StoreState};

/// Semantic memory result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticResult {
    /// Knowledge fact ID: a decimal string in JSON, read back from a string
    /// or a number.
    #[serde(
        serialize_with = "serde_id::serialize_id_as_string",
        deserialize_with = "serde_id::deserialize_id_from_string_or_number"
    )]
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Knowledge content text.
    pub content: String,
}

/// Semantic Memory for AI agents in WASM.
///
/// Stores knowledge facts as vectors with similarity search. Fact content text
/// is kept in the underlying [`VectorStore`] payload (mirroring the core
/// `SemanticMemory`) rather than in a separate map, so the payload is the single
/// source of truth for content while the store is live.
///
/// # Durability
///
/// **Not auto-persisted.** The WASM crate has no `persistence` feature, but the
/// `VectorStore` binary format (`export_to_bytes`/`save`/`load`, v3) **does**
/// serialize payloads, so fact content **survives** an explicit `save()` →
/// `load()` roundtrip. Call `save()` (e.g. to IndexedDB) to keep state across
/// reloads; nothing is written automatically.
///
/// # Example (JavaScript)
///
/// ```javascript
/// import { SemanticMemory } from 'velesdb-wasm';
///
/// const memory = new SemanticMemory(384);
/// memory.store(1, "Paris is the capital of France", embedding);
/// const results = memory.query(queryEmbedding, 5);
/// ```
#[wasm_bindgen]
pub struct SemanticMemory {
    store: StoreState,
}

impl SemanticMemory {
    /// Reads the `content` text for `id` from the store payload.
    fn content_for(&self, id: u64) -> String {
        self.store
            .ids
            .iter()
            .position(|&x| x == id)
            .and_then(|idx| self.store.payloads.get(idx))
            .and_then(Option::as_ref)
            .and_then(|p| p.get("content"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    }
}

#[wasm_bindgen]
impl SemanticMemory {
    /// Creates a new `SemanticMemory` with the given embedding dimension.
    #[wasm_bindgen(constructor)]
    pub fn new(dimension: usize) -> Result<SemanticMemory, JsValue> {
        let store = StoreState::new(dimension, "cosine")?;
        Ok(Self { store })
    }

    /// Stores a knowledge fact with its embedding vector.
    ///
    /// The content text is kept in the point payload as `{"content": ...}`.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this fact
    /// * `content` - Text content of the knowledge
    /// * `embedding` - Vector representation (`Float32Array`)
    #[wasm_bindgen]
    pub fn store(&mut self, id: u64, content: &str, embedding: &[f32]) -> Result<(), JsValue> {
        crate::store_search::validate_ingest_vector(embedding, self.store.dimension)
            .map_err(crate::wasm_error::WasmError::into_js_value)?;
        let payload = serde_json::json!({ "content": content });
        store_insert::insert_with_payload(&mut self.store, id, embedding, Some(payload));
        Ok(())
    }

    /// Queries semantic memory by similarity search.
    ///
    /// Returns a JSON array of `{id, score, content}` objects, `id` a decimal
    /// string.
    #[wasm_bindgen]
    pub fn query(&self, embedding: &[f32], top_k: usize) -> Result<JsValue, JsValue> {
        let semantic_results: Vec<SemanticResult> = self
            .store
            .top_k(embedding, top_k)?
            .into_iter()
            .map(|(id, score)| SemanticResult {
                id,
                score,
                content: self.content_for(id),
            })
            .collect();

        crate::js_value::to_js_value(&semantic_results)
            .map_err(|e| JsValue::from_str(&format!("Serialize error: {e}")))
    }

    /// Returns the number of stored knowledge facts.
    #[wasm_bindgen]
    pub fn len(&self) -> usize {
        self.store.ids.len()
    }

    /// Returns true if no knowledge facts are stored.
    #[wasm_bindgen]
    pub fn is_empty(&self) -> bool {
        self.store.ids.is_empty()
    }

    /// Deletes a knowledge fact by ID. Returns true if a fact was removed.
    #[wasm_bindgen]
    pub fn delete(&mut self, id: u64) -> bool {
        self.store.remove(id)
    }

    /// Removes a knowledge fact by ID.
    ///
    /// Deprecated alias for [`Self::delete`], kept for backward compatibility
    /// and naming parity with prior WASM releases.
    #[wasm_bindgen]
    pub fn remove(&mut self, id: u64) -> bool {
        self.delete(id)
    }

    /// Clears all knowledge facts.
    #[wasm_bindgen]
    pub fn clear(&mut self) {
        self.store.clear();
    }

    /// Returns the embedding dimension.
    #[wasm_bindgen]
    pub fn dimension(&self) -> usize {
        self.store.dimension()
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
