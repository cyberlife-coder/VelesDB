//! Vector similarity search methods for Collection.

use super::resolve;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::SearchResult;
use crate::quantization::{
    distance_pq_l2, pq_adc_batch_rescore, PQVector, ProductQuantizer, StorageMode,
};
use crate::scored_result::ScoredResult;
use crate::validation::validate_dimension_match;

// Re-export constants moved to vector_filter.rs for test compatibility.
#[cfg(test)]
pub(crate) use super::vector_filter::SELECTIVITY_THRESHOLD;

/// Estimates the real selectivity from a pre-filter bitmap.
///
/// Returns the ratio `bitmap.len() / collection_len` as an `f64` in
/// `[0.0, 1.0]`. Returns `0.0` when `collection_len` is zero.
#[allow(clippy::cast_precision_loss)]
#[inline]
pub(crate) fn estimate_real_selectivity(
    bitmap: &roaring::RoaringBitmap,
    collection_len: usize,
) -> f64 {
    if collection_len == 0 {
        return 0.0;
    }
    bitmap.len() as f64 / collection_len as f64
}

/// Tags each `SearchResult` with a `vector_score` component equal to its score.
///
/// For pure vector search, the HNSW/PQ score IS the vector component.
pub(super) fn tag_vector_component_scores(results: &mut [SearchResult]) {
    for result in results {
        result.component_scores = Some(smallvec::smallvec![("vector_score", result.score),]);
    }
}

impl Collection {
    fn search_ids_with_adc_if_pq(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        let config = self.config.read();
        let is_pq = matches!(config.storage_mode, StorageMode::ProductQuantization);
        let higher_is_better = config.metric.higher_is_better();
        let metric = config.metric;
        // u32 → usize: safe on all 32-bit+ targets (u32::MAX fits in usize).
        #[allow(clippy::cast_possible_truncation)]
        let oversampling = config.pq_rescore_oversampling.unwrap_or(0) as usize;
        drop(config);

        if !is_pq || oversampling == 0 {
            let results = self.index.search(query, k);
            return self.merge_delta(results, query, k, metric);
        }

        let candidates_k = k.saturating_mul(oversampling).max(k + 32);
        let index_results = self.index.search(query, candidates_k);
        let rescored =
            self.rescore_pq_candidates(query, k, metric, higher_is_better, index_results);
        self.merge_delta(rescored, query, k, metric)
    }

    /// Rescores PQ candidates using the product quantizer cache.
    ///
    /// For Euclidean metric with enough candidates, uses SIMD-accelerated
    /// batch ADC via [`pq_adc_batch_rescore`]. Falls back to per-item
    /// scalar scoring for small batches or non-Euclidean metrics.
    fn rescore_pq_candidates(
        &self,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
        higher_is_better: bool,
        index_results: Vec<ScoredResult>,
    ) -> Vec<ScoredResult> {
        let pq_cache = self.pq_cache.read();
        let quantizer = self.pq_quantizer.read();
        let Some(quantizer) = quantizer.as_ref() else {
            return index_results.into_iter().take(k).collect();
        };

        let mut rescored = if metric == DistanceMetric::Euclidean {
            rescore_euclidean_batch(query, quantizer, &pq_cache, &index_results)
        } else {
            rescore_per_item(query, quantizer, metric, &pq_cache, &index_results)
        };

        resolve::sort_scored_by_metric(&mut rescored, higher_is_better);
        rescored.truncate(k);
        rescored
    }

    /// Merges HNSW results with delta buffer and deferred indexer (if active).
    ///
    /// When both the delta buffer and deferred indexer are inactive, this is
    /// a no-op that returns results unchanged.
    #[cfg(feature = "persistence")]
    #[inline]
    pub(crate) fn merge_delta(
        &self,
        results: Vec<ScoredResult>,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
    ) -> Vec<ScoredResult> {
        let after_delta = crate::collection::streaming::merge_with_delta_scored(
            results,
            &self.delta_buffer,
            query,
            k,
            metric,
        );
        self.merge_deferred_search(after_delta, query, k, metric)
    }

    /// Merges search results with the deferred indexer buffer.
    ///
    /// No-op when deferred indexing is not configured or has no searchable data.
    #[cfg(feature = "persistence")]
    fn merge_deferred_search(
        &self,
        results: Vec<ScoredResult>,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
    ) -> Vec<ScoredResult> {
        let Some(ref di) = self.deferred_indexer else {
            return results;
        };
        if !di.is_searchable() {
            return results;
        }
        let hnsw_tuples: Vec<(u64, f32)> = results.into_iter().map(Into::into).collect();
        let merged = di.merge_with_hnsw(hnsw_tuples, query, k, metric);
        merged.into_iter().map(ScoredResult::from).collect()
    }

    #[cfg(not(feature = "persistence"))]
    #[inline]
    pub(crate) fn merge_delta(
        &self,
        results: Vec<ScoredResult>,
        _query: &[f32],
        _k: usize,
        _metric: DistanceMetric,
    ) -> Vec<ScoredResult> {
        results
    }
}

fn rescore_with_metric(
    query: &[f32],
    pq_vec: &PQVector,
    quantizer: &ProductQuantizer,
    metric: DistanceMetric,
) -> Result<f32> {
    if metric == DistanceMetric::Euclidean {
        Ok(distance_pq_l2(query, pq_vec, quantizer))
    } else {
        // reconstruct() returns a vector in OPQ-rotated space when a rotation matrix is
        // present. Apply the same rotation to the query so both operands are in the same
        // space before computing the metric. apply_rotation is a no-op (Cow::Borrowed) when
        // rotation is None, so this adds no overhead for standard PQ.
        let rotated_query = quantizer.apply_rotation(query);
        let reconstructed = quantizer.reconstruct(pq_vec)?;
        Ok(metric.calculate(&rotated_query, &reconstructed))
    }
}

/// Batch SIMD-accelerated ADC rescoring for Euclidean metric.
///
/// Collects PQ vectors from the cache, computes distances in one SIMD batch
/// via [`pq_adc_batch_rescore`], and maps scores back to candidates.
/// Candidates without a cached PQ vector keep their original HNSW score.
fn rescore_euclidean_batch(
    query: &[f32],
    quantizer: &ProductQuantizer,
    pq_cache: &std::collections::HashMap<u64, PQVector>,
    index_results: &[ScoredResult],
) -> Vec<ScoredResult> {
    // Collect candidates that have cached PQ codes; the rest keep their HNSW score.
    let mut with_pq: Vec<(usize, &PQVector)> = Vec::with_capacity(index_results.len());

    for (i, sr) in index_results.iter().enumerate() {
        if let Some(pq_vec) = pq_cache.get(&sr.id) {
            with_pq.push((i, pq_vec));
        }
    }

    // Start with original scores; overwrite those we can rescore.
    let mut scores: Vec<ScoredResult> = index_results
        .iter()
        .map(|sr| ScoredResult::new(sr.id, sr.score))
        .collect();

    if with_pq.is_empty() {
        return scores;
    }

    let pq_refs: Vec<&PQVector> = with_pq.iter().map(|&(_, pq)| pq).collect();

    match pq_adc_batch_rescore(quantizer, query, &pq_refs) {
        Ok(batch_distances) => {
            for (batch_idx, &(orig_idx, _)) in with_pq.iter().enumerate() {
                scores[orig_idx] =
                    ScoredResult::new(index_results[orig_idx].id, batch_distances[batch_idx]);
            }
        }
        Err(err) => {
            // ADC batch failed (should not happen with valid quantizer);
            // fall back to per-item scalar scoring.
            tracing::warn!(%err, "batch ADC rescore failed; falling back to scalar");
            for &(orig_idx, pq_vec) in &with_pq {
                scores[orig_idx] = ScoredResult::new(
                    index_results[orig_idx].id,
                    distance_pq_l2(query, pq_vec, quantizer),
                );
            }
        }
    }

    scores
}

/// Per-item rescoring for non-Euclidean metrics (cosine, dot product).
///
/// Reconstructs the full vector from PQ codes and computes the metric
/// in the (optionally OPQ-rotated) space.
fn rescore_per_item(
    query: &[f32],
    quantizer: &ProductQuantizer,
    metric: DistanceMetric,
    pq_cache: &std::collections::HashMap<u64, PQVector>,
    index_results: &[ScoredResult],
) -> Vec<ScoredResult> {
    index_results
        .iter()
        .map(|sr| {
            let score = pq_cache.get(&sr.id).map_or(sr.score, |pq_vec| {
                rescore_with_metric(query, pq_vec, quantizer, metric).unwrap_or_else(|err| {
                    tracing::warn!(sr.id, %err, "PQ rescore failed; using HNSW score");
                    sr.score
                })
            });
            ScoredResult::new(sr.id, score)
        })
        .collect()
}

impl Collection {
    /// Shared search-pipeline prologue: rejects metadata-only collections,
    /// validates the query dimension against the collection config, and reads
    /// the configured distance metric in a single lock scope.
    ///
    /// Factored from `search_with_ef` / `search_with_quality` /
    /// `search_with_forced_rerank` / `search_with_quality_no_rerank` for #452.
    /// Centralising the `metadata_only` check here (Devin #616 feedback)
    /// makes the 4 methods return a clean `Error::SearchNotSupported` instead
    /// of failing deeper inside the HNSW index on metadata-only collections —
    /// matching the behaviour of the inherent `search()` method.
    /// `#[inline]` preserves pre-refactor inlining (Phase 3.2 learning).
    #[inline]
    pub(super) fn validate_query_and_read_metric(&self, query: &[f32]) -> Result<DistanceMetric> {
        let config = self.config.read();
        if config.metadata_only {
            return Err(Error::SearchNotSupported(config.name.clone()));
        }
        validate_dimension_match(config.dimension, query.len())?;
        Ok(config.metric)
    }

    /// Shared search-pipeline epilogue: merges delta buffer, hydrates points and
    /// payloads, then tags each result with its `vector_score` component.
    ///
    /// Factored from `search_with_ef` / `search_with_quality` /
    /// `search_with_forced_rerank` / `search_with_quality_no_rerank` for #452.
    /// Marked `#[inline]` so rustc preserves the pre-refactor inlining decisions
    /// across the now-extracted call boundary (Phase 3.2 learning).
    #[inline]
    pub(super) fn finalize_search_results(
        &self,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
        index_results: Vec<ScoredResult>,
    ) -> Vec<SearchResult> {
        let index_results = self.merge_delta(index_results, query, k, metric);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results =
            resolve::resolve_scored_results(&index_results, &*vector_storage, &*payload_storage);
        tag_vector_component_scores(&mut results);
        results
    }

    /// Searches for the k nearest neighbors of the query vector.
    ///
    /// Uses HNSW index for fast approximate nearest neighbor search.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection,
    /// or if this is a metadata-only collection (use `query()` instead).
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let config = self.config.read();

        // Metadata-only collections don't support vector search
        if config.metadata_only {
            return Err(Error::SearchNotSupported(config.name.clone()));
        }

        validate_dimension_match(config.dimension, query.len())?;
        drop(config);

        // Use HNSW index for fast ANN search
        let index_results = self.search_ids_with_adc_if_pq(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results =
            resolve::resolve_scored_results(&index_results, &*vector_storage, &*payload_storage);
        tag_vector_component_scores(&mut results);
        Ok(results)
    }

    /// Performs vector similarity search with custom `ef_search` parameter.
    ///
    /// Higher `ef_search` = better recall, slower search.
    /// Default `ef_search` is 128 (Balanced mode).
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_with_ef(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.validate_query_and_read_metric(query)?;

        // Convert ef_search to SearchQuality
        let quality = match ef_search {
            0..=64 => crate::SearchQuality::Fast,
            65..=128 => crate::SearchQuality::Balanced,
            129..=512 => crate::SearchQuality::Accurate,
            _ => crate::SearchQuality::Perfect,
        };

        let index_results = self.index.search_with_quality(query, k, quality)?;
        Ok(self.finalize_search_results(query, k, metric, index_results))
    }

    /// Performs vector similarity search with a specific [`SearchQuality`] profile.
    ///
    /// Use this instead of [`search_with_ef`] for named quality modes like
    /// [`SearchQuality::AutoTune`] that compute ef dynamically.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: crate::SearchQuality,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.validate_query_and_read_metric(query)?;

        let index_results = self.index.search_with_quality(query, k, quality)?;
        Ok(self.finalize_search_results(query, k, metric, index_results))
    }

    /// Routes vector search through `QuerySearchOptions` from a WITH clause.
    ///
    /// Priority: `quality` (from `mode`) > `ef_search` > default `search()`.
    /// When `force_rerank` is `Some(true)`, applies explicit SIMD reranking
    /// regardless of quality mode. When `Some(false)`, suppresses automatic
    /// reranking even if the quality mode would enable it.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub(crate) fn search_with_opts(
        &self,
        query: &[f32],
        k: usize,
        opts: &crate::collection::search::query::QuerySearchOptions,
    ) -> Result<Vec<SearchResult>> {
        // When no options are set, fall back to default search.
        if opts.quality.is_none() && opts.ef_search.is_none() && opts.force_rerank.is_none() {
            return self.search(query, k);
        }

        // Resolve the search quality: explicit mode > ef_search bracket > default.
        let quality = opts.quality.unwrap_or_else(|| {
            opts.ef_search
                .map_or(crate::SearchQuality::Balanced, |ef| match ef {
                    0..=64 => crate::SearchQuality::Fast,
                    65..=128 => crate::SearchQuality::Balanced,
                    129..=512 => crate::SearchQuality::Accurate,
                    _ => crate::SearchQuality::Perfect,
                })
        });

        match opts.force_rerank {
            Some(true) => self.search_with_forced_rerank(query, k, quality),
            Some(false) => self.search_with_quality_no_rerank(query, k, quality),
            None => self.search_with_quality(query, k, quality),
        }
    }

    /// Searches with forced SIMD reranking regardless of quality mode.
    fn search_with_forced_rerank(
        &self,
        query: &[f32],
        k: usize,
        quality: crate::SearchQuality,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.validate_query_and_read_metric(query)?;

        let rerank_k = k.saturating_mul(4).max(k + 32);
        let index_results = self
            .index
            .search_with_rerank_quality(query, k, rerank_k, quality)?;
        Ok(self.finalize_search_results(query, k, metric, index_results))
    }

    /// Searches with a quality profile but suppresses two-stage reranking.
    ///
    /// Uses `search_hnsw_only` via the ef_search derived from the quality profile,
    /// skipping the automatic reranking that `search_with_quality` would enable.
    fn search_with_quality_no_rerank(
        &self,
        query: &[f32],
        k: usize,
        quality: crate::SearchQuality,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.validate_query_and_read_metric(query)?;

        let ef_search = quality.ef_search(k);
        let index_results = self.index.search_hnsw_only(query, k, ef_search);
        Ok(self.finalize_search_results(query, k, metric, index_results))
    }

    /// Performs fast vector similarity search returning only IDs and scores.
    ///
    /// Perf: This is ~3-5x faster than `search()` because it skips vector/payload retrieval.
    /// Use this when you only need IDs and scores, not full point data.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of [`ScoredResult`] sorted by similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_ids(&self, query: &[f32], k: usize) -> Result<Vec<ScoredResult>> {
        // Rejects metadata-only + validates dimension in one lock scope.
        // Metric is unused here (search_ids_with_adc_if_pq re-reads config)
        // but reusing the helper keeps the metadata_only guard consistent
        // with the 4 other dispatch paths (#452 Devin #616 feedback).
        let _metric = self.validate_query_and_read_metric(query)?;

        // Perf: Direct HNSW search without vector/payload retrieval
        let results = self.search_ids_with_adc_if_pq(query, k);
        Ok(results)
    }

    /// Searches for the k nearest neighbors with metadata filtering.
    ///
    /// Performs post-filtering: retrieves more candidates from HNSW,
    /// then filters by metadata conditions.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Maximum number of results to return
    /// * `filter` - Metadata filter to apply
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.validate_query_and_read_metric(query)?;
        let higher_is_better = metric.higher_is_better();

        let candidates_k = super::vector_filter::compute_oversampled_k(k, filter);

        // Attempt bitmap pre-filter from secondary indexes.
        let index_results =
            self.search_with_optional_bitmap(query, k, candidates_k, filter, metric);

        Ok(self.filter_and_hydrate(index_results, filter, k, higher_is_better))
    }

    /// Searches HNSW with an optional bitmap pre-filter from secondary indexes.
    fn search_with_optional_bitmap(
        &self,
        query: &[f32],
        k: usize,
        candidates_k: usize,
        filter: &crate::filter::Filter,
        metric: DistanceMetric,
    ) -> Vec<ScoredResult> {
        if let Some(bitmap) = self.build_prefilter_bitmap(filter) {
            let ef_search = candidates_k.max(k * 10);
            let results =
                self.index
                    .search_hnsw_only_filtered(query, candidates_k, ef_search, &bitmap);
            return self.merge_delta(results, query, candidates_k, metric);
        }
        self.search_ids_with_adc_if_pq(query, candidates_k)
    }
}
