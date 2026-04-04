//! Persistence and serialization methods for `VectorStore`.
//!
//! Handles export/import to binary format and IndexedDB save/load/delete.

use wasm_bindgen::prelude::*;

use crate::{persistence, serialization};

use super::vector_store::VectorStore;

#[wasm_bindgen]
impl VectorStore {
    /// Exports to binary format for IndexedDB/localStorage.
    #[wasm_bindgen]
    pub fn export_to_bytes(&self) -> Result<Vec<u8>, JsValue> {
        Ok(serialization::export_to_bytes(self))
    }

    /// Saves to `IndexedDB`.
    #[wasm_bindgen]
    pub async fn save(&self, db_name: &str) -> Result<(), JsValue> {
        let bytes = self.export_to_bytes()?;
        persistence::save_to_indexeddb(db_name, &bytes).await
    }

    /// Loads from `IndexedDB`.
    #[wasm_bindgen]
    pub async fn load(db_name: &str) -> Result<VectorStore, JsValue> {
        let bytes = persistence::load_from_indexeddb(db_name).await?;
        Self::import_from_bytes(&bytes)
    }

    /// Deletes `IndexedDB` database.
    #[wasm_bindgen]
    pub async fn delete_database(db_name: &str) -> Result<(), JsValue> {
        persistence::delete_database(db_name).await
    }

    /// Imports from binary format.
    #[wasm_bindgen]
    pub fn import_from_bytes(bytes: &[u8]) -> Result<VectorStore, JsValue> {
        serialization::import_from_bytes(bytes)
    }

    // ========================================================================
    // Sparse search (v1.5)
    // ========================================================================

    /// Inserts a sparse vector into the internal sparse index.
    ///
    /// Lazily initializes the sparse index on first call.
    #[wasm_bindgen]
    pub fn sparse_insert(
        &mut self,
        doc_id: u64,
        indices: &[u32],
        values: &[f32],
    ) -> Result<(), JsValue> {
        let idx = self
            .sparse_index
            .get_or_insert_with(crate::sparse::SparseIndex::new);
        idx.insert(doc_id, indices, values)
    }

    /// Searches the internal sparse index.
    ///
    /// Returns a JSON array of `{doc_id, score}` objects sorted by score descending.
    #[wasm_bindgen]
    pub fn sparse_search(
        &self,
        indices: &[u32],
        values: &[f32],
        k: usize,
    ) -> Result<JsValue, JsValue> {
        match &self.sparse_index {
            Some(idx) => idx.search(indices, values, k),
            None => {
                // No sparse data inserted yet — return empty results
                let empty: Vec<()> = vec![];
                serde_wasm_bindgen::to_value(&empty)
                    .map_err(|e| JsValue::from_str(&e.to_string()))
            }
        }
    }
}
