//! WASM bindings for sparse vector search.
//!
//! A thin binding over core's in-memory sparse index
//! (`velesdb_core::sparse_index`, compiled without the `persistence`
//! feature): the browser stores, replaces, removes and scores sparse
//! vectors with the engine's own code, so it ranks exactly as core does.

use serde::{Deserialize, Serialize};
use velesdb_core::sparse_index::{sparse_search, SparseInvertedIndex, SparseVector};
use wasm_bindgen::prelude::*;

/// Result from sparse search.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SparseSearchResult {
    doc_id: u64,
    score: f32,
}

impl SparseSearchResult {
    /// Returns the document id of this result. Test-only accessor used to
    /// assert ranking order; the wasm path serializes the fields directly.
    #[cfg(test)]
    pub(crate) fn doc_id(&self) -> u64 {
        self.doc_id
    }
}

/// Builds core's `SparseVector` from the parallel arrays a JS caller passes,
/// refusing mismatched lengths and — as core's ingest and query boundaries
/// do — a non-finite weight. `what` names the arrays in the error.
fn sparse_vector(what: &str, indices: &[u32], values: &[f32]) -> Result<SparseVector, String> {
    if indices.len() != values.len() {
        return Err(format!(
            "{what}indices/values length mismatch: {} vs {}",
            indices.len(),
            values.len()
        ));
    }
    let vector = SparseVector::new(
        indices
            .iter()
            .copied()
            .zip(values.iter().copied())
            .collect(),
    );
    velesdb_core::validation::validate_sparse_vector_is_finite(&vector)
        .map_err(|e| e.to_string())?;
    Ok(vector)
}

/// In-memory sparse inverted index for WASM, backed by core's
/// [`SparseInvertedIndex`].
#[wasm_bindgen]
#[derive(Default)]
pub struct SparseIndex {
    inner: SparseInvertedIndex,
}

#[wasm_bindgen]
impl SparseIndex {
    /// Creates a new empty sparse index.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a document with the given sparse vector.
    ///
    /// `indices` and `values` must have the same length and every weight
    /// must be finite. Duplicate indices are summed and zero weights dropped,
    /// as core does. Re-inserting a `doc_id` replaces its vector: terms the
    /// new vector drops stop matching.
    #[wasm_bindgen]
    pub fn insert(&mut self, doc_id: u64, indices: &[u32], values: &[f32]) -> Result<(), JsValue> {
        let vector = sparse_vector("", indices, values).map_err(|e| JsValue::from_str(&e))?;
        self.inner.insert(doc_id, &vector);
        Ok(())
    }

    /// Removes a document's postings from the index.
    ///
    /// Returns `true` when the document was present.
    #[wasm_bindgen]
    pub fn remove(&mut self, doc_id: u64) -> bool {
        let present = self.inner.contains(doc_id);
        self.inner.delete(doc_id);
        present
    }

    /// Searches the index with the given sparse query vector.
    ///
    /// Returns a JSON array of `{doc_id, score}` objects, sorted by score descending.
    #[wasm_bindgen]
    pub fn search(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<JsValue, JsValue> {
        let results = self
            .search_scored(query_indices, query_values, k)
            .map_err(|e| JsValue::from_str(&e))?;
        crate::js_value::to_js_value(&results)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
    }

    /// Native-testable scoring kernel shared by [`Self::search`] and the
    /// `VectorStore::search_sparse` delegate: core's `sparse_search`, top-`k`
    /// by inner product, without touching `JsValue`.
    pub(crate) fn search_scored(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<Vec<SparseSearchResult>, String> {
        let query = sparse_vector("query ", query_indices, query_values)?;
        Ok(sparse_search(&self.inner, &query, k)
            .into_iter()
            .map(|doc| SparseSearchResult {
                doc_id: doc.doc_id,
                score: doc.score,
            })
            .collect())
    }

    /// Returns the number of documents in the index.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn doc_count(&self) -> usize {
        usize::try_from(self.inner.doc_count()).unwrap_or(usize::MAX)
    }
}

/// Fuses pre-computed dense and sparse search results using Reciprocal Rank Fusion (RRF).
///
/// Both `dense_results` and `sparse_results` should be JSON arrays of `[doc_id, score]` pairs.
/// Returns a JSON array of `{doc_id, score}` objects, sorted by fused score descending,
/// truncated to the top `k` entries.
#[wasm_bindgen]
pub fn hybrid_search_fuse(
    dense_results: JsValue,
    sparse_results: JsValue,
    rrf_k: u32,
    k: usize,
) -> Result<JsValue, JsValue> {
    let dense: Vec<(u64, f32)> = serde_wasm_bindgen::from_value(dense_results)
        .map_err(|e| JsValue::from_str(&format!("Invalid dense_results: {e}")))?;
    let sparse: Vec<(u64, f32)> = serde_wasm_bindgen::from_value(sparse_results)
        .map_err(|e| JsValue::from_str(&format!("Invalid sparse_results: {e}")))?;

    // Delegate to the canonical RRF in velesdb-core so the browser engine
    // reproduces core's ranking 1:1 (single source of truth for fusion math)
    // instead of re-deriving the formula here.
    let fused = velesdb_core::FusionStrategy::RRF { k: rrf_k }
        .fuse(vec![dense, sparse])
        .map_err(|e| JsValue::from_str(&format!("Fusion error: {e}")))?;

    let mut results: Vec<SparseSearchResult> = fused
        .into_iter()
        .map(|(doc_id, score)| SparseSearchResult { doc_id, score })
        .collect();
    results.truncate(k);

    crate::js_value::to_js_value(&results)
        .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
}

#[cfg(test)]
#[path = "sparse_tests.rs"]
mod tests;
