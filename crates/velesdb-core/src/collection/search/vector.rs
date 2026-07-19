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
        let config = self.storage.config.read();
        let is_pq = matches!(config.storage_mode, StorageMode::ProductQuantization);
        let higher_is_better = config.metric.higher_is_better();
        let metric = config.metric;
        // u32 → usize: safe on all 32-bit+ targets (u32::MAX fits in usize).
        #[allow(clippy::cast_possible_truncation)]
        let oversampling = config.pq_rescore_oversampling.unwrap_or(0) as usize;
        drop(config);

        if !is_pq || oversampling == 0 {
            let results = self.storage.index.search(query, k);
            return self.merge_delta(results, query, k, metric);
        }

        let candidates_k = k.saturating_mul(oversampling).max(k + 32);
        let index_results = self.storage.index.search(query, candidates_k);
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
        let pq_cache = self.storage.pq_cache.read();
        let quantizer = self.storage.pq_quantizer.read();
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
            &self.streaming.delta_buffer,
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
        let Some(ref di) = self.streaming.deferred_indexer else {
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
            // ADC batch rejected the candidates (e.g. an out-of-range PQ code on a
            // tampered/corrupt persisted vector). The same codes would also drive the
            // scalar `distance_pq_l2` path out of bounds (LUT indexing panic), so we
            // do NOT re-invoke it here. The affected candidates keep their original
            // HNSW score (already seeded into `scores`) — a clean skip, matching the
            // graceful degradation used for cache misses above.
            tracing::warn!(%err, "batch ADC rescore rejected candidates; keeping HNSW scores");
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
        let config = self.storage.config.read();
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

        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();

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
        let config = self.storage.config.read();

        // Metadata-only collections don't support vector search
        if config.metadata_only {
            return Err(Error::SearchNotSupported(config.name.clone()));
        }

        validate_dimension_match(config.dimension, query.len())?;
        drop(config);

        // Use HNSW index for fast ANN search
        let index_results = self.search_ids_with_adc_if_pq(query, k);

        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();

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

        // Convert ef_search to a value-preserving SearchQuality.
        let quality = super::vector_filter::ef_to_quality(ef_search);

        let index_results = self.storage.index.search_with_quality(query, k, quality)?;
        Ok(self.finalize_search_results(query, k, metric, index_results))
    }

    /// Rejects a Perfect (brute-force) search on a collection larger than the
    /// configured `max_perfect_mode_vectors` cap (parity item E).
    ///
    /// Gates at search entry — before `index.search_with_quality` — so the
    /// HNSW/SIMD inner loop is never touched on a violation. A no-op for every
    /// non-Perfect quality mode. Shared by [`Self::search_with_quality`],
    /// `search_with_opts`, and the filtered path `search_with_filter_and_opts`
    /// so all four index entry points apply the identical gate.
    ///
    /// # Errors
    ///
    /// Returns [`Error::GuardRail`] when `quality` is
    /// [`SearchQuality::Perfect`](crate::SearchQuality::Perfect) and the
    /// indexed vector count exceeds the cap.
    pub(super) fn enforce_perfect_mode_limit(&self, quality: crate::SearchQuality) -> Result<()> {
        if !matches!(quality, crate::SearchQuality::Perfect) {
            return Ok(());
        }
        let cap = self.runtime_limits().max_perfect_mode_vectors;
        let size = self.storage.index.len();
        if size > cap {
            return Err(Error::GuardRail(format!(
                "Perfect (brute-force) search rejected: collection has {size} vectors, \
                 exceeding max_perfect_mode_vectors cap of {cap}; raise \
                 `limits.max_perfect_mode_vectors` in VelesConfig or use a lower quality mode"
            )));
        }
        Ok(())
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
        self.enforce_perfect_mode_limit(quality)?;

        let index_results = self.storage.index.search_with_quality(query, k, quality)?;
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

        // Resolve the search quality: explicit mode > exact ef_search > default.
        let quality = opts.quality.unwrap_or_else(|| {
            opts.ef_search.map_or(
                crate::SearchQuality::Balanced,
                super::vector_filter::ef_to_quality,
            )
        });

        // Parity item E: gate Perfect-mode over-cap once here, covering the
        // forced-rerank / no-rerank branches that bypass `search_with_quality`.
        self.enforce_perfect_mode_limit(quality)?;

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
            .storage
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

        // Issue #699 follow-up: align with HnswIndex::search_with_quality which
        // uses ef_search_for_scale. Without scaling, this no-rerank path produced
        // a smaller ef than search_with_quality on >10K datasets, breaking the
        // implicit contract that "no rerank" only suppresses reranking, not the
        // candidate-pool sizing.
        let ef_search = quality.ef_search_for_scale(k, self.storage.index.len());
        let index_results = self.storage.index.search_hnsw_only(query, k, ef_search);
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
            let results = self.storage.index.search_hnsw_only_filtered(
                query,
                candidates_k,
                ef_search,
                &bitmap,
            );
            return self.merge_delta(results, query, candidates_k, metric);
        }
        self.search_ids_with_adc_if_pq(query, candidates_k)
    }
}

#[cfg(test)]
mod tests {
    use super::{rescore_euclidean_batch, PQVector, ProductQuantizer};
    use crate::scored_result::ScoredResult;
    use std::collections::HashMap;

    fn small_trained_pq() -> ProductQuantizer {
        let vectors = vec![
            vec![1.0, 2.0, 3.0, 4.0],
            vec![5.0, 6.0, 7.0, 8.0],
            vec![-1.0, -2.0, 9.0, 10.0],
        ];
        ProductQuantizer::train(&vectors, 2, 2).expect("train small PQ")
    }

    #[test]
    fn invalid_pq_code_in_search_path_skips_candidate_without_panic() {
        // Routing an out-of-range PQ code through the Euclidean batch scoring entry
        // point (the same one the fallback uses) must NOT panic and must NOT re-invoke
        // the unvalidated scalar indexing path. The candidate keeps its HNSW score.
        let quantizer = small_trained_pq();
        // num_centroids == 2, so code 99 is out of range for both subspaces.
        let bad = PQVector { codes: vec![0, 99] };

        let mut pq_cache: HashMap<u64, PQVector> = HashMap::new();
        pq_cache.insert(7, bad);

        let index_results = vec![ScoredResult::new(7, 0.42)];
        let query = vec![1.0, 2.0, 3.0, 4.0];

        let scored = rescore_euclidean_batch(&query, &quantizer, &pq_cache, &index_results);

        assert_eq!(scored.len(), 1);
        assert_eq!(scored[0].id, 7);
        // Clean skip: original HNSW score is retained, no panic, no garbage.
        assert!(
            (scored[0].score - 0.42).abs() < 1e-6,
            "rejected candidate must keep its HNSW score, got {}",
            scored[0].score
        );
    }
}
