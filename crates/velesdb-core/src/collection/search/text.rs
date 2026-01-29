//! Text and hybrid search methods for Collection.

use super::OrderedFloat;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

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
    #[must_use]
    pub fn text_search(&self, query: &str, k: usize) -> Vec<SearchResult> {
        let bm25_results = self.text_index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        bm25_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect()
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
    #[must_use]
    pub fn text_search_with_filter(
        &self,
        query: &str,
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Vec<SearchResult> {
        // Retrieve more candidates for filtering
        let candidates_k = k.saturating_mul(4).max(k + 10);
        let bm25_results = self.text_index.search(query, candidates_k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        bm25_results
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
                };

                Some(SearchResult::new(point, score))
            })
            .take(k)
            .collect()
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
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let config = self.config.read();
        if vector_query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: vector_query.len(),
            });
        }
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;

        // Get vector search results (more than k to allow for fusion)
        let vector_results = self.index.search(vector_query, k * 2);

        // Get BM25 text search results
        let text_results = self.text_index.search(text_query, k * 2);

        // Perf: Apply RRF with FxHashMap for faster hashing
        // RRF score = weight / (rank + 60) - the constant 60 is standard (Cormack et al.)
        let mut fused_scores: rustc_hash::FxHashMap<u64, f32> =
            rustc_hash::FxHashMap::with_capacity_and_hasher(
                vector_results.len() + text_results.len(),
                rustc_hash::FxBuildHasher,
            );

        // Add vector scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in vector_results.iter().enumerate() {
            let rrf_score = weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Add text scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in text_results.iter().enumerate() {
            let rrf_score = text_weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Perf: Streaming top-k with BinaryHeap (O(n log k) vs O(n log n) for full sort)
        // Use min-heap of size k: always keep the k highest scores
        let mut top_k: BinaryHeap<Reverse<(OrderedFloat, u64)>> = BinaryHeap::with_capacity(k + 1);

        for (id, score) in fused_scores {
            top_k.push(Reverse((OrderedFloat(score), id)));
            if top_k.len() > k {
                top_k.pop(); // Remove smallest
            }
        }

        // Extract and sort descending
        let mut scored_ids: Vec<(u64, f32)> = top_k
            .into_iter()
            .map(|Reverse((OrderedFloat(s), id))| (id, s))
            .collect();
        scored_ids.sort_by(|a, b| b.1.total_cmp(&a.1));

        // Fetch full point data
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = scored_ids
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect();

        Ok(results)
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
        if vector_query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: vector_query.len(),
            });
        }
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;

        // Get more candidates for filtering
        let candidates_k = k.saturating_mul(4).max(k + 10);

        // Get vector search results
        let vector_results = self.index.search(vector_query, candidates_k);

        // Get BM25 text search results
        let text_results = self.text_index.search(text_query, candidates_k);

        // Apply RRF (Reciprocal Rank Fusion)
        let mut fused_scores: rustc_hash::FxHashMap<u64, f32> = rustc_hash::FxHashMap::default();

        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in vector_results.iter().enumerate() {
            let rrf_score = weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in text_results.iter().enumerate() {
            let rrf_score = text_weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Sort by fused score
        let mut scored_ids: Vec<_> = fused_scores.into_iter().collect();
        scored_ids.sort_by(|a, b| b.1.total_cmp(&a.1));

        // Fetch full point data and apply filter
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = scored_ids
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
                };

                Some(SearchResult::new(point, score))
            })
            .take(k)
            .collect();

        Ok(results)
    }
}
