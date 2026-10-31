//! Batch and multi-query search methods for Collection.

use super::pending::PendingRestriction;
use super::resolve;
use crate::collection::expiry::now_unix_secs;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::point::SearchResult;
use crate::scored_result::ScoredResult;
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
        filters: &[Option<Filter>],
    ) -> Result<Vec<Vec<SearchResult>>> {
        let metric = self.validate_filtered_batch(queries, filters)?;
        let candidates_k = k.saturating_mul(4).max(k + 10);
        let restrictions = filters
            .iter()
            .map(|filter| PendingRestriction::NONE.post_filtered_if(filter.is_some()));
        // Merged on this thread before the storage guards below are taken:
        // the merge reads storage itself, chunk by chunk, and must do so
        // neither under a guard this thread holds nor on a rayon worker.
        let mut merged =
            self.batch_search_merging_pending(queries, candidates_k, metric, restrictions)?;
        for candidates in &mut merged {
            resolve::sort_scored_by_metric(candidates, metric.higher_is_better());
        }

        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let vs: &dyn VectorStorage = &*vector_storage;
        let ps: &dyn PayloadStorage = &*payload_storage;
        let now_secs = now_unix_secs();

        // Parallel filter + resolve across queries (P0 QPS optimization)
        let all_results = merged
            .par_iter()
            .zip(filters.par_iter())
            .map(|(candidates, filter)| {
                Self::filter_and_resolve_batch(candidates, filter.as_ref(), k, now_secs, vs, ps)
            })
            .collect();
        drop(payload_storage);
        drop(vector_storage);

        Ok(all_results)
    }

    /// Checks a filtered batch's shape and query dimensions, returning the
    /// collection's metric.
    fn validate_filtered_batch(
        &self,
        queries: &[&[f32]],
        filters: &[Option<Filter>],
    ) -> Result<crate::DistanceMetric> {
        if queries.len() != filters.len() {
            return Err(Error::Config(format!(
                "Queries count ({}) does not match filters count ({})",
                queries.len(),
                filters.len()
            )));
        }
        let (dimension, metric) = {
            let cfg = self.storage.config.read();
            (cfg.dimension, cfg.metric)
        };
        for query in queries {
            validate_dimension_match(dimension, query.len())?;
        }
        Ok(metric)
    }

    /// Resolves one query's best-first candidates into its first `k` results
    /// whose payload `filter` matches (a point without payload is judged as
    /// `null`). Payloads are judged before any vector is copied, so only the
    /// returned points' vectors are read.
    fn filter_and_resolve_batch(
        candidates: &[ScoredResult],
        filter: Option<&Filter>,
        k: usize,
        now_secs: u64,
        vector_storage: &dyn VectorStorage,
        payload_storage: &dyn PayloadStorage,
    ) -> Vec<SearchResult> {
        let pairs = candidates.iter().map(|sr| (sr.id, sr.score));
        resolve::hydrate_accepted(pairs, k, now_secs, vector_storage, payload_storage, |p| {
            filter.is_none_or(|f| f.matches(p.unwrap_or(&serde_json::Value::Null)))
        })
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
        let (dimension, metric) = {
            let cfg = self.storage.config.read();
            (cfg.dimension, cfg.metric)
        };

        // Validate all query dimensions first
        for query in queries {
            validate_dimension_match(dimension, query.len())?;
        }

        // Perf: Use parallel HNSW search (P0 optimization)
        let restrictions = std::iter::repeat(PendingRestriction::NONE);
        let merged = self.batch_search_merging_pending(queries, k, metric, restrictions)?;

        // Parallel result resolution across queries (P0 QPS optimization)
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let vs: &dyn VectorStorage = &*vector_storage;
        let ps: &dyn PayloadStorage = &*payload_storage;

        let results: Vec<Vec<SearchResult>> = merged
            .par_iter()
            .map(|query_results| resolve::resolve_scored_results(query_results, vs, ps))
            .collect();
        drop(payload_storage);
        drop(vector_storage);

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
    /// Vector of `SearchResult` sorted best-first (descending fused score for
    /// similarity metrics, ascending fused distance for Euclidean/Hamming).
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

        let restriction = PendingRestriction::NONE.post_filtered_if(filter.is_some());
        let batch_results =
            self.search_batch_merging_pending(vectors, overfetch_k, metric, restriction)?;
        let filtered = self.apply_pre_fusion_filter(batch_results, filter);

        // Every branch carries this collection's metric scores, so score-level
        // strategies must honor the metric's polarity (#2102).
        let directions = vec![metric.score_direction(); filtered.len()];
        let fused = fusion
            .fuse_with_directions(filtered, &directions)
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

        let config = self.storage.config.read();
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

    /// Runs the batch index search and merges, per query, the points not yet
    /// in the index (async builder, deferred indexer) as
    /// [`merge_pending`](Self::merge_pending) does, each query keeping
    /// `overfetch_k` best `(id, score)` pairs.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if any query vector has a dimension
    /// different from the index dimension.
    fn search_batch_merging_pending(
        &self,
        vectors: &[&[f32]],
        overfetch_k: usize,
        metric: crate::DistanceMetric,
        restriction: PendingRestriction<'_>,
    ) -> Result<Vec<Vec<(u64, f32)>>> {
        let restrictions = std::iter::repeat(restriction);
        let merged =
            self.batch_search_merging_pending(vectors, overfetch_k, metric, restrictions)?;
        Ok(merged
            .into_iter()
            .map(|results| results.into_iter().map(Into::into).collect())
            .collect())
    }

    /// The batch form of [`search_merging_pending`](Self::search_merging_pending):
    /// one pending snapshot before the parallel index search, then, per query,
    /// a refetch while unindexed ids crowd its `k` (in parallel) and the merge
    /// under that query's restriction (on the caller; one restriction per
    /// query, in order).
    ///
    /// # Errors
    ///
    /// Returns what the index search returns.
    fn batch_search_merging_pending<'r>(
        &self,
        queries: &[&[f32]],
        k: usize,
        metric: crate::DistanceMetric,
        restrictions: impl Iterator<Item = PendingRestriction<'r>>,
    ) -> Result<Vec<Vec<ScoredResult>>> {
        let quality = self.runtime_search_quality();
        let pending = self.snapshot_pending();
        let index_results = self
            .storage
            .index
            .search_batch_parallel(queries, k, quality)?;
        // Only the refetches run in parallel: each is an index search, which
        // takes no collection lock. The merge scores pending points under the
        // `vector_storage` read guard, so it runs here, on the caller: a
        // global rayon worker taking that guard parks behind any queued
        // writer, while the batch's own resolve phase joins the global pool
        // holding it — no worker ever frees, and the process stops.
        let refetched: Vec<Vec<ScoredResult>> = index_results
            .into_par_iter()
            .zip(queries.par_iter())
            .map(|(first, query)| {
                Self::index_results_past_pending(&pending, k, first, |n| {
                    self.storage.index.search_with_quality(query, n, quality)
                })
            })
            .collect::<Result<_>>()?;
        Ok(refetched
            .into_iter()
            .zip(queries)
            .zip(restrictions)
            .map(|((results, query), restriction)| {
                self.merge_pending(results, &pending, query, k, metric, restriction)
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
        let payload_storage = self.storage.payload_storage.read();
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
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();

        resolve::resolve_id_score_pairs(fused, top_k, &*vector_storage, &*payload_storage)
    }

    /// Performs multi-query search returning only IDs and fused scores.
    ///
    /// This is a faster variant of `multi_query_search` that skips fetching
    /// vector and payload data. Use when you only need document IDs.
    ///
    /// Reuses [`validate_multi_query_inputs`](Self::validate_multi_query_inputs),
    /// [`overfetch_factor`](Self::overfetch_factor), and
    /// [`search_batch_merging_pending`](Self::search_batch_merging_pending) to eliminate
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

        let batch_results = self.search_batch_merging_pending(
            vectors,
            overfetch_k,
            metric,
            PendingRestriction::NONE,
        )?;

        // Same polarity contract as `multi_query_search` (#2102).
        let directions = vec![metric.score_direction(); batch_results.len()];
        let fused = fusion
            .fuse_with_directions(batch_results, &directions)
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        Ok(fused.into_iter().take(top_k).collect())
    }
}
