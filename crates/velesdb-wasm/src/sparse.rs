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

    // RRF: score(d) = sum over lists of 1 / (k + rank_in_list)
    // N-06: rrf_k is a smoothing constant typically in [10, 100]; the precision
    // loss from u32 → f32 is negligible (< 1 ULP for values < 2^24).
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
    results.truncate(k);

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
    fn test_sparse_index_upsert_does_not_increment_doc_count() {
        let mut index = SparseIndex::new();
        // First insert: new doc → count becomes 1.
        index.insert(42, &[1, 2], &[1.0, 0.5]).unwrap();
        assert_eq!(
            index.doc_count(),
            1,
            "first insert should increment doc_count"
        );

        // Second insert of the same doc_id with overlapping terms → still 1.
        index.insert(42, &[1, 3], &[2.0, 0.3]).unwrap();
        assert_eq!(
            index.doc_count(),
            1,
            "re-insert of existing doc_id must not increment doc_count"
        );

        // Third insert of the same doc_id with a completely disjoint term set → still 1.
        index.insert(42, &[99], &[0.7]).unwrap();
        assert_eq!(
            index.doc_count(),
            1,
            "re-insert with disjoint terms must not increment doc_count"
        );

        // A different doc_id → count becomes 2.
        index.insert(99, &[1], &[1.0]).unwrap();
        assert_eq!(
            index.doc_count(),
            2,
            "new doc_id should increment doc_count"
        );
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
