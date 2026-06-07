//! `AgentMemory` WASM bindings (EPIC-016 US-003)
//!
//! Provides semantic memory for AI agents in the browser.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::{store_insert, VectorStore};

/// Semantic memory result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticResult {
    /// Knowledge fact ID.
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
/// **In-memory only.** The WASM crate has no `persistence` feature. The
/// `VectorStore` binary format used by `export_to_bytes`/`save`/`load` does
/// **not** serialize payloads, so fact content does **not** survive a
/// store reload. Persist content out-of-band (e.g. in application state or
/// IndexedDB) if durability is required.
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
    store: VectorStore,
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
        let store = VectorStore::new(dimension, "cosine")?;
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
        crate::store_search::validate_dimension(embedding.len(), self.store.dimension)?;
        let payload = serde_json::json!({ "content": content });
        store_insert::insert_with_payload(&mut self.store, id, embedding, Some(payload));
        Ok(())
    }

    /// Queries semantic memory by similarity search.
    ///
    /// Returns a JSON array of {id, score, content} objects.
    #[wasm_bindgen]
    pub fn query(&self, embedding: &[f32], top_k: usize) -> Result<JsValue, JsValue> {
        let results_js = self.store.search(embedding, top_k)?;

        // Parse search results and enrich with content
        let results_str = results_js
            .as_string()
            .ok_or_else(|| JsValue::from_str("Invalid search results"))?;

        let search_results: Vec<crate::SearchResult> = serde_json::from_str(&results_str)
            .map_err(|e| JsValue::from_str(&format!("Parse error: {e}")))?;

        let semantic_results: Vec<SemanticResult> = search_results
            .into_iter()
            .map(|r| SemanticResult {
                id: r.id,
                score: r.score,
                content: self.content_for(r.id),
            })
            .collect();

        serde_wasm_bindgen::to_value(&semantic_results)
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
mod tests {
    use super::*;

    #[test]
    fn test_semantic_memory_new() {
        let memory = SemanticMemory::new(384).unwrap();
        assert_eq!(memory.dimension(), 384);
        assert!(memory.is_empty());
    }

    #[test]
    fn test_semantic_memory_store_and_len() {
        let mut memory = SemanticMemory::new(4).unwrap();
        let embedding = vec![0.1, 0.2, 0.3, 0.4];

        memory.store(1, "Test content", &embedding).unwrap();

        assert_eq!(memory.len(), 1);
        assert!(!memory.is_empty());
    }

    #[test]
    fn test_semantic_memory_content_in_payload() {
        let mut memory = SemanticMemory::new(4).unwrap();
        let embedding = vec![0.1, 0.2, 0.3, 0.4];

        memory.store(1, "Paris is the capital", &embedding).unwrap();

        assert_eq!(memory.content_for(1), "Paris is the capital");
    }

    #[test]
    fn test_semantic_memory_delete() {
        let mut memory = SemanticMemory::new(4).unwrap();
        let embedding = vec![0.1, 0.2, 0.3, 0.4];

        memory.store(1, "Test content", &embedding).unwrap();
        assert_eq!(memory.len(), 1);

        let removed = memory.delete(1);
        assert!(removed);
        assert!(memory.is_empty());
    }

    #[test]
    fn test_semantic_memory_clear() {
        let mut memory = SemanticMemory::new(4).unwrap();
        let embedding = vec![0.1, 0.2, 0.3, 0.4];

        memory.store(1, "Content 1", &embedding).unwrap();
        memory.store(2, "Content 2", &embedding).unwrap();
        assert_eq!(memory.len(), 2);

        memory.clear();
        assert!(memory.is_empty());
    }
}
