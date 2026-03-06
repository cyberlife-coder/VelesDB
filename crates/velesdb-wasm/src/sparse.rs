//! WASM bindings for sparse vector search.
//!
//! Provides in-memory sparse index operations for browser-side use.
//! Self-contained implementation that does not depend on the `persistence` feature gate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

/// Result from sparse search.
#[derive(Serialize, Deserialize)]
struct SparseSearchResult {
    doc_id: u64,
    score: f32,
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
    /// Number of documents inserted.
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
        self.doc_count += 1;
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
        if query_indices.len() != query_values.len() {
            return Err(JsValue::from_str(&format!(
                "query indices/values length mismatch: {} vs {}",
                query_indices.len(),
                query_values.len()
            )));
        }
        if k == 0 {
            let empty: Vec<SparseSearchResult> = Vec::new();
            return serde_wasm_bindgen::to_value(&empty)
                .map_err(|e| JsValue::from_str(&e.to_string()));
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

        serde_wasm_bindgen::to_value(&results)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
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
/// Returns a JSON array of `{doc_id, score}` objects, sorted by fused score descending.
#[wasm_bindgen]
pub fn hybrid_search_fuse(
    dense_results: JsValue,
    sparse_results: JsValue,
    rrf_k: u32,
) -> Result<JsValue, JsValue> {
    let dense: Vec<(u64, f32)> = serde_wasm_bindgen::from_value(dense_results)
        .map_err(|e| JsValue::from_str(&format!("Invalid dense_results: {e}")))?;
    let sparse: Vec<(u64, f32)> = serde_wasm_bindgen::from_value(sparse_results)
        .map_err(|e| JsValue::from_str(&format!("Invalid sparse_results: {e}")))?;

    // RRF: score(d) = sum over lists of 1 / (k + rank_in_list)
    let k_f32 = rrf_k as f32;
    let mut scores: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();

    for (rank, &(doc_id, _)) in dense.iter().enumerate() {
        *scores.entry(doc_id).or_insert(0.0) += 1.0 / (k_f32 + (rank as f32) + 1.0);
    }
    for (rank, &(doc_id, _)) in sparse.iter().enumerate() {
        *scores.entry(doc_id).or_insert(0.0) += 1.0 / (k_f32 + (rank as f32) + 1.0);
    }

    let mut results: Vec<SparseSearchResult> = scores
        .into_iter()
        .map(|(doc_id, score)| SparseSearchResult { doc_id, score })
        .collect();
    results.sort_by(|a, b| b.score.total_cmp(&a.score));

    serde_wasm_bindgen::to_value(&results)
        .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sparse_index_insert_search_basic() {
        let mut index = SparseIndex::new();
        // Insert 5 documents
        index.insert(1, &[10, 20, 30], &[1.0, 0.5, 0.3]).unwrap();
        index.insert(2, &[10, 40], &[0.8, 1.2]).unwrap();
        index.insert(3, &[20, 30, 50], &[0.9, 0.7, 0.4]).unwrap();
        index.insert(4, &[10, 20], &[0.3, 1.5]).unwrap();
        index.insert(5, &[30, 40, 50], &[1.0, 0.6, 0.2]).unwrap();

        assert_eq!(index.doc_count(), 5);

        // Manually test accumulation for query = {10: 1.0, 20: 1.0}
        // Doc 1: 1.0*1.0 + 0.5*1.0 = 1.5
        // Doc 2: 0.8*1.0 = 0.8
        // Doc 3: 0.9*1.0 = 0.9
        // Doc 4: 0.3*1.0 + 1.5*1.0 = 1.8
        let mut accum: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
        let query_terms: &[(u32, f32)] = &[(10, 1.0), (20, 1.0)];
        for &(term_id, q_w) in query_terms {
            if let Some(list) = index.postings.get(&term_id) {
                for &(doc_id, d_w) in list {
                    *accum.entry(doc_id).or_insert(0.0) += q_w * d_w;
                }
            }
        }
        let mut results: Vec<(u64, f32)> = accum.into_iter().collect();
        results.sort_by(|a, b| b.1.total_cmp(&a.1));

        assert_eq!(results[0].0, 4); // Doc 4 = 1.8
        assert_eq!(results[1].0, 1); // Doc 1 = 1.5
    }

    #[test]
    fn test_sparse_index_empty() {
        let index = SparseIndex::new();
        assert_eq!(index.doc_count(), 0);
    }

    #[test]
    fn test_sparse_index_insert_works() {
        let mut index = SparseIndex::new();
        // Verify correct insert works with matching lengths.
        assert!(index.insert(1, &[10, 20], &[1.0, 2.0]).is_ok());
        assert_eq!(index.doc_count(), 1);
    }

    #[test]
    fn test_rrf_fusion_basic() {
        // Test RRF logic manually (can't call wasm function in native tests).
        let k_f32 = 60.0_f32;
        let dense: &[(u64, f32)] = &[(1_u64, 0.9_f32), (2, 0.8), (3, 0.7)];
        let sparse: &[(u64, f32)] = &[(2_u64, 5.0_f32), (3, 4.0), (4, 3.0)];

        let mut scores: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
        for (rank, &(doc_id, _)) in dense.iter().enumerate() {
            *scores.entry(doc_id).or_insert(0.0) += 1.0 / (k_f32 + (rank as f32) + 1.0);
        }
        for (rank, &(doc_id, _)) in sparse.iter().enumerate() {
            *scores.entry(doc_id).or_insert(0.0) += 1.0 / (k_f32 + (rank as f32) + 1.0);
        }

        let mut results: Vec<(u64, f32)> = scores.into_iter().collect();
        results.sort_by(|a, b| b.1.total_cmp(&a.1));

        // Doc 2 appears in both lists (rank 1 in dense, rank 0 in sparse) -> highest RRF
        assert_eq!(results[0].0, 2);
        // Doc 3 also in both
        assert_eq!(results[1].0, 3);
    }
}
