//! Text and hybrid search methods for Collection.

use super::resolve;
use super::OrderedFloat;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;

impl Collection {
    /// Performs full-text search using BM25.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score (descending).
    ///
    /// # Errors
    ///
    /// Returns an error if storage retrieval fails.
    pub fn text_search(&self, query: &str, k: usize) -> Result<Vec<SearchResult>> {
        let bm25_results = self.text_index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        Ok(resolve::resolve_id_score_pairs(
            &bm25_results,
            bm25_results.len(),
            &*vector_storage,
            &*payload_storage,
        ))
    }

    /// Performs full-text search with metadata filtering.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `k` - Maximum number of results to return
    /// * `filter` - Metadata filter to apply
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score (descending).
    ///
    /// # Errors
    ///
    /// Returns an error if storage retrieval fails.
    pub fn text_search_with_filter(
        &self,
        query: &str,
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        // Retrieve more candidates for filtering
        let candidates_k = k.saturating_mul(4).max(k + 10);
        let bm25_results = self.text_index.search(query, candidates_k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        Ok(bm25_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                // Apply filter - if no payload, filter fails
                let payload_ref = payload.as_ref()?;
                if !filter.matches(payload_ref) {
                    return None;
                }

                let point = Point {
                    id,
                    vector,
                    payload,
                    sparse_vectors: None,
                };

                Some(SearchResult::new(point, score))
            })
            .take(k)
            .collect())
    }

    /// Performs hybrid search combining vector similarity and full-text search.
    ///
    /// Uses Reciprocal Rank Fusion (RRF) to combine results from both searches.
    ///
    /// # Arguments
    ///
    /// * `vector_query` - Query vector for similarity search
    /// * `text_query` - Text query for BM25 search
    /// * `k` - Maximum number of results to return
    /// * `vector_weight` - Weight for vector results (0.0-1.0, default 0.5)
    ///
    /// # Performance (v0.9+)
    ///
    /// - **Streaming RRF**: `BinaryHeap` maintains top-k during fusion (O(n log k) vs O(n log n))
    /// - **Vector-first gating**: Text search limited to 2k candidates for efficiency
    /// - **`FxHashMap`**: Faster hashing for score aggregation
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub fn hybrid_search(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        use crate::index::VectorIndex;

        let config = self.config.read();
        validate_dimension_match(config.dimension, vector_query.len())?;
        let metric = config.metric;
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;

        let overfetch_k = k * 2;
        let raw_vector_results = self.index.search(vector_query, overfetch_k);
        let vector_results =
            self.merge_delta(raw_vector_results, vector_query, overfetch_k, metric);
        let text_results = self.text_index.search(text_query, k * 2);

        let fused_scores =
            Self::compute_rrf_scores(&vector_results, &text_results, weight, text_weight);

        let scored_ids = Self::top_k_from_scores(fused_scores, k);
        Ok(self.resolve_scored_ids(&scored_ids))
    }

    /// Computes RRF fused scores from vector and text search results.
    #[allow(clippy::cast_precision_loss)]
    fn compute_rrf_scores(
        vector_results: &[crate::scored_result::ScoredResult],
        text_results: &[(u64, f32)],
        vector_weight: f32,
        text_weight: f32,
    ) -> rustc_hash::FxHashMap<u64, f32> {
        let mut fused: rustc_hash::FxHashMap<u64, f32> =
            rustc_hash::FxHashMap::with_capacity_and_hasher(
                vector_results.len() + text_results.len(),
                rustc_hash::FxBuildHasher,
            );
        for (rank, sr) in vector_results.iter().enumerate() {
            *fused.entry(sr.id).or_insert(0.0) += vector_weight / (rank as f32 + 60.0);
        }
        for (rank, (id, _)) in text_results.iter().enumerate() {
            *fused.entry(*id).or_insert(0.0) += text_weight / (rank as f32 + 60.0);
        }
        fused
    }

    /// Extracts top-k IDs from fused scores using a streaming min-heap.
    fn top_k_from_scores(
        fused_scores: rustc_hash::FxHashMap<u64, f32>,
        k: usize,
    ) -> Vec<(u64, f32)> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let mut heap: BinaryHeap<Reverse<(OrderedFloat, u64)>> = BinaryHeap::with_capacity(k + 1);
        for (id, score) in fused_scores {
            heap.push(Reverse((OrderedFloat(score), id)));
            if heap.len() > k {
                heap.pop();
            }
        }
        let mut scored: Vec<(u64, f32)> = heap
            .into_iter()
            .map(|Reverse((OrderedFloat(s), id))| (id, s))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored
    }

    /// Resolves scored IDs to full `SearchResult` with point data.
    fn resolve_scored_ids(&self, scored_ids: &[(u64, f32)]) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        resolve::resolve_id_score_pairs(
            scored_ids,
            scored_ids.len(),
            &*vector_storage,
            &*payload_storage,
        )
    }

    /// Performs hybrid search (vector + text) with metadata filtering.
    ///
    /// Uses Reciprocal Rank Fusion (RRF) to combine results from both searches,
    /// then applies metadata filter.
    ///
    /// # Arguments
    ///
    /// * `vector_query` - Query vector for similarity search
    /// * `text_query` - Text query for BM25 search
    /// * `k` - Maximum number of results to return
    /// * `vector_weight` - Weight for vector results (0.0-1.0, default 0.5)
    /// * `filter` - Metadata filter to apply
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub fn hybrid_search_with_filter(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        use crate::index::VectorIndex;

        let config = self.config.read();
        validate_dimension_match(config.dimension, vector_query.len())?;
        let metric = config.metric;
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;
        let candidates_k = k.saturating_mul(4).max(k + 10);

        let raw_vector_results = self.index.search(vector_query, candidates_k);
        let vector_results =
            self.merge_delta(raw_vector_results, vector_query, candidates_k, metric);
        let text_results = self.text_index.search(text_query, candidates_k);

        let fused_scores =
            Self::compute_rrf_scores(&vector_results, &text_results, weight, text_weight);

        let mut scored_ids: Vec<_> = fused_scores.into_iter().collect();
        scored_ids.sort_by(|a, b| b.1.total_cmp(&a.1));

        Ok(self.resolve_scored_ids_filtered(&scored_ids, filter, k))
    }

    /// Resolves scored IDs to `SearchResult` with metadata filter applied.
    fn resolve_scored_ids_filtered(
        &self,
        scored_ids: &[(u64, f32)],
        filter: &crate::filter::Filter,
        k: usize,
    ) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        scored_ids
            .iter()
            .filter_map(|&(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();
                let payload_ref = payload.as_ref()?;
                if !filter.matches(payload_ref) {
                    return None;
                }
                Some(SearchResult::new(
                    Point {
                        id,
                        vector,
                        payload,
                        sparse_vectors: None,
                    },
                    score,
                ))
            })
            .take(k)
            .collect()
    }
}
