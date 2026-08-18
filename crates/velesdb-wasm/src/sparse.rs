//! WASM bindings for sparse vector search.
//!
//! Provides in-memory sparse index operations for browser-side use.
//! Self-contained implementation that does not depend on the `persistence` feature gate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

/// A sparse vector: sorted parallel arrays of term indices and weights.
struct SparseVec {
    indices: Vec<u32>,
    values: Vec<f32>,
}

impl SparseVec {
    fn new(mut pairs: Vec<(u32, f32)>) -> Self {
        pairs.sort_by_key(|&(idx, _)| idx);
        // Merge duplicates, filter zeros
        let mut indices = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());
        if pairs.is_empty() {
            return Self { indices, values };
        }
        let mut cur_idx = pairs[0].0;
        let mut cur_val = pairs[0].1;
        for &(idx, val) in &pairs[1..] {
            if idx == cur_idx {
                cur_val += val;
            } else {
                if cur_val.abs() >= f32::EPSILON {
                    indices.push(cur_idx);
                    values.push(cur_val);
                }
                cur_idx = idx;
                cur_val = val;
            }
        }
        if cur_val.abs() >= f32::EPSILON {
            indices.push(cur_idx);
            values.push(cur_val);
        }
        Self { indices, values }
    }
}

/// In-memory sparse inverted index for WASM.
///
/// Uses a `BTreeMap<u32, Vec<(u64, f32)>>` as posting lists.
#[wasm_bindgen]
pub struct SparseIndex {
    /// term_id -> list of (doc_id, weight), sorted by doc_id.
    postings: BTreeMap<u32, Vec<(u64, f32)>>,
    /// Max weight per term (for MaxScore pruning).
    max_weights: BTreeMap<u32, f32>,
    /// Set of all doc_ids that have been inserted at least once.
    ///
    /// This is the authoritative source for "is this doc already in the index?"
    /// — checking posting lists is unreliable when a re-insert touches different
    /// terms than the original insert (disjoint-term upsert case).
    known_docs: std::collections::BTreeSet<u64>,
    /// Number of distinct documents (= `known_docs.len()`).
    doc_count: usize,
}

impl Default for SparseIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl SparseIndex {
    /// Creates a new empty sparse index.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            postings: BTreeMap::new(),
            max_weights: BTreeMap::new(),
            known_docs: std::collections::BTreeSet::new(),
            doc_count: 0,
        }
    }

    /// Inserts a document with the given sparse vector.
    ///
    /// `indices` and `values` must have the same length.
    #[wasm_bindgen]
    pub fn insert(&mut self, doc_id: u64, indices: &[u32], values: &[f32]) -> Result<(), JsValue> {
        if indices.len() != values.len() {
            return Err(JsValue::from_str(&format!(
                "indices/values length mismatch: {} vs {}",
                indices.len(),
                values.len()
            )));
        }
        let pairs: Vec<(u32, f32)> = indices
            .iter()
            .copied()
            .zip(values.iter().copied())
            .collect();
        let sv = SparseVec::new(pairs);

        // `known_docs` is the authoritative set of all doc_ids ever inserted.
        // This is O(log n) and correctly handles the disjoint-term re-insert case
        // (where the new vector touches different terms than the original insert).
        let is_new_doc = !self.known_docs.contains(&doc_id);

        for (&term_id, &weight) in sv.indices.iter().zip(sv.values.iter()) {
            let list = self.postings.entry(term_id).or_default();
            match list.binary_search_by_key(&doc_id, |&(id, _)| id) {
                Ok(pos) => list[pos] = (doc_id, weight),
                Err(pos) => list.insert(pos, (doc_id, weight)),
            }
            let max_w = self.max_weights.entry(term_id).or_insert(0.0);
            if weight.abs() > *max_w {
                *max_w = weight.abs();
            }
        }
        // Only increment doc_count for genuinely new documents, not for re-inserts/updates.
        if is_new_doc {
            self.known_docs.insert(doc_id);
            self.doc_count += 1;
        }
        Ok(())
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
        serde_wasm_bindgen::to_value(&results)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
    }

    /// Native-testable scoring kernel shared by [`Self::search`] and the
    /// `VectorStore::search_sparse` delegate.
    ///
    /// Performs DAAT (Document-At-A-Time) accumulation and top-`k` extraction
    /// without touching `JsValue`, so it can be exercised off-`wasm32`.
    pub(crate) fn search_scored(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<Vec<SparseSearchResult>, String> {
        if query_indices.len() != query_values.len() {
            return Err(format!(
                "query indices/values length mismatch: {} vs {}",
                query_indices.len(),
                query_values.len()
            ));
        }
        if k == 0 {
            return Ok(Vec::new());
        }

        // DAAT (Document-At-A-Time) accumulation using a hash map.
        let mut accum: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();

        for (&term_id, &q_weight) in query_indices.iter().zip(query_values.iter()) {
            if let Some(list) = self.postings.get(&term_id) {
                for &(doc_id, d_weight) in list {
                    *accum.entry(doc_id).or_insert(0.0) += q_weight * d_weight;
                }
            }
        }

        // Top-k extraction.
        let mut results: Vec<SparseSearchResult> = accum
            .into_iter()
            .map(|(doc_id, score)| SparseSearchResult { doc_id, score })
            .collect();
        results.sort_by(|a, b| b.score.total_cmp(&a.score));
        results.truncate(k);
        Ok(results)
    }

    /// Returns the number of documents in the index.
    #[wasm_bindgen(getter)]
    pub fn doc_count(&self) -> usize {
        self.doc_count
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

    serde_wasm_bindgen::to_value(&results)
        .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
}

#[cfg(test)]
#[path = "sparse_tests.rs"]
mod tests;
