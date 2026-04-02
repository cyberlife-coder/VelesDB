//! Batch and multi-query search methods for Collection.

use super::resolve;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::SearchQuality;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;
use rayon::prelude::*;

impl Collection {
    /// Performs batch search for multiple query vectors in parallel with metadata filtering.
    /// Supports a different filter for each query in the batch.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    /// * `filters` - List of optional filters (must match queries length)
    ///
    /// # Returns
    ///
    /// Vector of search results for each query, matching its respective filter.
    ///
    /// # Errors
    ///
    /// Returns an error if queries and filters have different lengths or dimension mismatch.
    pub fn search_batch_with_filters(
        &self,
        queries: &[&[f32]],
        k: usize,
        filters: &[Option<crate::filter::Filter>],
    ) -> Result<Vec<Vec<SearchResult>>> {
        if queries.len() != filters.len() {
            return Err(Error::Config(format!(
                "Queries count ({}) does not match filters count ({})",
                queries.len(),
                filters.len()
            )));
        }

        let dimension = self.config.read().dimension;
        for query in queries {
            validate_dimension_match(dimension, query.len())?;
        }

        let candidates_k = k.saturating_mul(4).max(k + 10);
        let metric = self.config.read().metric;
        let higher_is_better = metric.higher_is_better();
        let index_results =
            self.index
                .search_batch_parallel(queries, candidates_k, SearchQuality::Balanced)?;

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        // Pre-merge delta per query (requires &self, safe for rayon via shared ref)
        let merged: Vec<_> = index_results
            .into_iter()
            .zip(queries.iter())
            .map(|(qr, query)| self.merge_delta(qr, query, candidates_k, metric))
            .collect();

        // Parallel filter + resolve across queries (P0 QPS optimization)
        let vs: &dyn VectorStorage = &*vector_storage;
        let ps: &dyn PayloadStorage = &*payload_storage;

        let all_results: Vec<Vec<SearchResult>> = merged
            .par_iter()
            .zip(filters.par_iter())
            .map(|(query_results, filter_opt)| {
                let mut filtered =
                    Self::filter_and_resolve_batch(query_results, filter_opt.as_ref(), vs, ps);
                resolve::sort_results_by_metric(&mut filtered, higher_is_better);
                filtered.truncate(k);
                filtered
            })
            .collect();

        Ok(all_results)
    }

    /// Filters and resolves a single batch query's results.
    fn filter_and_resolve_batch(
        results: &[crate::scored_result::ScoredResult],
        filter: Option<&crate::filter::Filter>,
        vector_storage: &dyn VectorStorage,
        payload_storage: &dyn PayloadStorage,
    ) -> Vec<SearchResult> {
        results
            .iter()
            .filter_map(|sr| {
                let payload = payload_storage.retrieve(sr.id).ok().flatten();
                if let Some(f) = filter {
                    let matches = match payload.as_ref() {
                        Some(p) => f.matches(p),
                        None => f.matches(&serde_json::Value::Null),
                    };
                    if !matches {
                        return None;
                    }
                }
                let vector = vector_storage.retrieve(sr.id).ok().flatten()?;
                Some(SearchResult::new(
                    Point {
                        id: sr.id,
                        vector,
                        payload,
                        sparse_vectors: None,
                    },
                    sr.score,
                ))
            })
            .collect()
    }

    /// Performs batch search for multiple query vectors in parallel with a single metadata filter.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    /// * `filter` - Metadata filter to apply to all results
    ///
    /// # Errors
    ///
    /// Returns an error if any query has incorrect dimension.
    pub fn search_batch_with_filter(
        &self,
        queries: &[&[f32]],
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<Vec<SearchResult>>> {
        let filters: Vec<Option<crate::filter::Filter>> = vec![Some(filter.clone()); queries.len()];
        self.search_batch_with_filters(queries, k, &filters)
    }

    /// Performs batch search for multiple query vectors in parallel.
    ///
    /// This method is optimized for high throughput using parallel index traversal.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    ///
    /// # Returns
    ///
    /// Vector of search results for each query, with full point data.
    ///
    /// # Errors
    ///
    /// Returns an error if any query vector dimension doesn't match the collection.
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
    ) -> Result<Vec<Vec<SearchResult>>> {
        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        // Validate all query dimensions first
        for query in queries {
            validate_dimension_match(dimension, query.len())?;
        }

        // Perf: Use parallel HNSW search (P0 optimization)
        let metric = self.config.read().metric;
        let index_results =
            self.index
                .search_batch_parallel(queries, k, SearchQuality::Balanced)?;

        // Pre-merge delta per query (requires &self)
        let merged: Vec<_> = index_results
            .into_iter()
            .zip(queries.iter())
            .map(|(qr, query)| self.merge_delta(qr, query, k, metric))
            .collect();

        // Parallel result resolution across queries (P0 QPS optimization)
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();
        let vs: &dyn VectorStorage = &*vector_storage;
        let ps: &dyn PayloadStorage = &*payload_storage;

        let results: Vec<Vec<SearchResult>> = merged
            .par_iter()
            .map(|query_results| resolve::resolve_scored_results(query_results, vs, ps))
            .collect();

        Ok(results)
    }

    /// Performs multi-query search with result fusion.
    ///
    /// This method executes parallel searches for multiple query vectors and fuses
    /// the results using the specified fusion strategy. Ideal for Multiple Query
    /// Generation (MQG) pipelines where multiple reformulations of a user query
    /// are searched simultaneously.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Slice of query vectors (all must have same dimension)
    /// * `top_k` - Maximum number of results to return after fusion
    /// * `fusion` - Strategy for combining results (Average, Maximum, RRF, Weighted)
    /// * `filter` - Optional metadata filter to apply to all queries
    ///
    /// # Returns
    ///
    /// Vector of `SearchResult` sorted by fused score descending.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `vectors` is empty
    /// - Any vector has incorrect dimension
    /// - More than 10 vectors are provided (configurable limit)
    #[allow(clippy::needless_pass_by_value)]
    pub fn multi_query_search(
        &self,
        vectors: &[&[f32]],
        top_k: usize,
        fusion: crate::fusion::FusionStrategy,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.validate_multi_query_inputs(vectors)?;
        let overfetch_k = Self::overfetch_factor(top_k);

        let batch_results = self.search_and_merge_delta(vectors, overfetch_k, metric)?;
        let filtered = self.apply_pre_fusion_filter(batch_results, filter);

        let fused = fusion
            .fuse(filtered)
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        Ok(self.hydrate_fused_results(&fused, top_k))
    }

    /// Validates inputs for `multi_query_search` and returns the distance metric.
    fn validate_multi_query_inputs(&self, vectors: &[&[f32]]) -> Result<crate::DistanceMetric> {
        const MAX_VECTORS: usize = 10;

        if vectors.is_empty() {
            return Err(Error::Config(
                "multi_query_search requires at least one vector".into(),
            ));
        }
        if vectors.len() > MAX_VECTORS {
            return Err(Error::Config(format!(
                "multi_query_search supports at most {MAX_VECTORS} vectors, got {}",
                vectors.len()
            )));
        }

        let config = self.config.read();
        let dimension = config.dimension;
        let metric = config.metric;
        drop(config);

        for vector in vectors {
            validate_dimension_match(dimension, vector.len())?;
        }

        Ok(metric)
    }

    /// Calculates the overfetch factor for better fusion quality.
    fn overfetch_factor(top_k: usize) -> usize {
        match top_k {
            0..=10 => top_k * 20,
            11..=50 => top_k * 10,
            51..=100 => top_k * 5,
            _ => top_k * 2,
        }
    }

    /// Runs batch index search and merges delta buffer per query.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if any query vector has a dimension
    /// different from the index dimension.
    fn search_and_merge_delta(
        &self,
        vectors: &[&[f32]],
        overfetch_k: usize,
        metric: crate::DistanceMetric,
    ) -> Result<Vec<Vec<(u64, f32)>>> {
        let batch_results = self.index.search_batch_parallel(
            vectors,
            overfetch_k,
            crate::SearchQuality::Balanced,
        )?;

        Ok(batch_results
            .into_iter()
            .zip(vectors)
            .map(|(query_results, query)| {
                self.merge_delta(query_results, query, overfetch_k, metric)
                    .into_iter()
                    .map(Into::into)
                    .collect()
            })
            .collect())
    }

    /// Applies metadata filter to batch results before fusion.
    fn apply_pre_fusion_filter(
        &self,
        batch_results: Vec<Vec<(u64, f32)>>,
        filter: Option<&crate::filter::Filter>,
    ) -> Vec<Vec<(u64, f32)>> {
        let Some(f) = filter else {
            return batch_results;
        };
        let payload_storage = self.payload_storage.read();
        batch_results
            .into_iter()
            .map(|query_results| {
                query_results
                    .into_iter()
                    .filter(|(id, _score)| {
                        if let Ok(Some(payload)) = payload_storage.retrieve(*id) {
                            f.matches(&payload)
                        } else {
                            false
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// Fetches full point data for the top-k fused results.
    fn hydrate_fused_results(&self, fused: &[(u64, f32)], top_k: usize) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        resolve::resolve_id_score_pairs(fused, top_k, &*vector_storage, &*payload_storage)
    }

    /// Performs multi-query search returning only IDs and fused scores.
    ///
    /// This is a faster variant of `multi_query_search` that skips fetching
    /// vector and payload data. Use when you only need document IDs.
    ///
    /// Reuses [`validate_multi_query_inputs`](Self::validate_multi_query_inputs),
    /// [`overfetch_factor`](Self::overfetch_factor), and
    /// [`search_and_merge_delta`](Self::search_and_merge_delta) to eliminate
    /// duplication with `multi_query_search`.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Slice of query vectors
    /// * `top_k` - Maximum number of results
    /// * `fusion` - Fusion strategy
    ///
    /// # Returns
    ///
    /// Vector of `(id, fused_score)` tuples sorted by score descending.
    ///
    /// # Errors
    ///
    /// Returns an error if vectors is empty, exceeds max limit, or has dimension mismatch.
    #[allow(clippy::needless_pass_by_value)]
    pub fn multi_query_search_ids(
        &self,
        vectors: &[&[f32]],
        top_k: usize,
        fusion: crate::fusion::FusionStrategy,
    ) -> Result<Vec<(u64, f32)>> {
        let metric = self.validate_multi_query_inputs(vectors)?;
        let overfetch_k = Self::overfetch_factor(top_k);

        let batch_results = self.search_and_merge_delta(vectors, overfetch_k, metric)?;

        let fused = fusion
            .fuse(batch_results)
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        Ok(fused.into_iter().take(top_k).collect())
    }
}
