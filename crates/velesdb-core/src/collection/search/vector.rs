//! Vector similarity search methods for Collection.

use super::resolve;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::{Point, SearchResult};
use crate::quantization::{distance_pq_l2, PQVector, ProductQuantizer, StorageMode};
use crate::scored_result::ScoredResult;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;

/// Tags each `SearchResult` with a `vector_score` component equal to its score.
///
/// For pure vector search, the HNSW/PQ score IS the vector component.
fn tag_vector_component_scores(results: &mut [SearchResult]) {
    for result in results {
        result.component_scores = Some(smallvec::smallvec![(
            "vector_score".to_string(),
            result.score
        ),]);
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

        let mut rescored: Vec<ScoredResult> = index_results
            .into_iter()
            .map(|sr| {
                let score = pq_cache.get(&sr.id).map_or(sr.score, |pq_vec| {
                    rescore_with_metric(query, pq_vec, quantizer, metric).unwrap_or_else(|err| {
                        tracing::warn!(sr.id, %err, "PQ rescore failed; using HNSW score");
                        sr.score
                    })
                });
                ScoredResult::new(sr.id, score)
            })
            .collect();

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

impl Collection {
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
        let config = self.config.read();

        validate_dimension_match(config.dimension, query.len())?;
        drop(config);

        // Convert ef_search to SearchQuality
        let quality = match ef_search {
            0..=64 => crate::SearchQuality::Fast,
            65..=128 => crate::SearchQuality::Balanced,
            129..=512 => crate::SearchQuality::Accurate,
            _ => crate::SearchQuality::Perfect,
        };

        let metric = self.config.read().metric;
        let index_results = self.index.search_with_quality(query, k, quality);
        let index_results = self.merge_delta(index_results, query, k, metric);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results =
            resolve::resolve_scored_results(&index_results, &*vector_storage, &*payload_storage);
        tag_vector_component_scores(&mut results);
        Ok(results)
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
        let config = self.config.read();
        validate_dimension_match(config.dimension, query.len())?;
        let metric = config.metric;
        drop(config);

        let index_results = self.index.search_with_quality(query, k, quality);
        let index_results = self.merge_delta(index_results, query, k, metric);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results =
            resolve::resolve_scored_results(&index_results, &*vector_storage, &*payload_storage);
        tag_vector_component_scores(&mut results);
        Ok(results)
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
        let config = self.config.read();
        validate_dimension_match(config.dimension, query.len())?;
        let metric = config.metric;
        drop(config);

        let rerank_k = k.saturating_mul(4).max(k + 32);
        let index_results = self
            .index
            .search_with_rerank_quality(query, k, rerank_k, quality);
        let index_results = self.merge_delta(index_results, query, k, metric);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results =
            resolve::resolve_scored_results(&index_results, &*vector_storage, &*payload_storage);
        tag_vector_component_scores(&mut results);
        Ok(results)
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
        let config = self.config.read();
        validate_dimension_match(config.dimension, query.len())?;
        let metric = config.metric;
        drop(config);

        let ef_search = quality.ef_search(k);
        let index_results = self.index.search_hnsw_only(query, k, ef_search);
        let index_results = self.merge_delta(index_results, query, k, metric);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results =
            resolve::resolve_scored_results(&index_results, &*vector_storage, &*payload_storage);
        tag_vector_component_scores(&mut results);
        Ok(results)
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
        let config = self.config.read();

        validate_dimension_match(config.dimension, query.len())?;
        drop(config);

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
        let config = self.config.read();
        validate_dimension_match(config.dimension, query.len())?;
        let higher_is_better = config.metric.higher_is_better();
        drop(config);

        let selectivity = estimate_filter_selectivity(filter);
        // Reason: k is a small search count (typically <1000); f64 has 52-bit mantissa.
        #[allow(clippy::cast_precision_loss)]
        let k_f64 = k as f64;
        #[allow(clippy::cast_precision_loss)]
        let lower = (k + 10) as f64;
        // Reason: result is clamped to [k+10, 10_000] so no truncation risk.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let candidates_k = (k_f64 / selectivity).ceil().clamp(lower, 10_000.0) as usize;
        let index_results = self.search_ids_with_adc_if_pq(query, candidates_k);

        Ok(self.filter_and_hydrate(index_results, filter, k, higher_is_better))
    }

    /// Searches with metadata filtering AND quality options from a WITH clause.
    ///
    /// Combines the candidate-retrieval strategy of [`search_with_opts`] with the
    /// post-filtering of [`search_with_filter`]. When quality options are present,
    /// uses quality-aware HNSW search (higher ef_search) for candidates before
    /// applying the metadata filter. Falls back to [`search_with_filter`] when no
    /// quality options are set.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub(crate) fn search_with_filter_and_opts(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
        opts: &crate::collection::search::query::QuerySearchOptions,
    ) -> Result<Vec<SearchResult>> {
        // No quality options: delegate to the standard filter path.
        if opts.quality.is_none() && opts.ef_search.is_none() && opts.force_rerank.is_none() {
            return self.search_with_filter(query, k, filter);
        }

        let config = self.config.read();
        validate_dimension_match(config.dimension, query.len())?;
        let higher_is_better = config.metric.higher_is_better();
        let metric = config.metric;
        drop(config);

        let quality = opts.quality.unwrap_or_else(|| {
            opts.ef_search
                .map_or(crate::SearchQuality::Balanced, |ef| match ef {
                    0..=64 => crate::SearchQuality::Fast,
                    65..=128 => crate::SearchQuality::Balanced,
                    129..=512 => crate::SearchQuality::Accurate,
                    _ => crate::SearchQuality::Perfect,
                })
        });

        // Over-fetch for filtering: retrieve more candidates than needed.
        let selectivity = estimate_filter_selectivity(filter);
        // Reason: k is a small search count (typically <1000); f64 has 52-bit mantissa.
        #[allow(clippy::cast_precision_loss)]
        let k_f64 = k as f64;
        #[allow(clippy::cast_precision_loss)]
        let lower = (k + 10) as f64;
        // Reason: result is clamped to [k+10, 10_000] so no truncation risk.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let candidates_k = (k_f64 / selectivity).ceil().clamp(lower, 10_000.0) as usize;

        let index_results = self.index.search_with_quality(query, candidates_k, quality);
        let index_results = self.merge_delta(index_results, query, candidates_k, metric);

        let results = self.filter_and_hydrate(index_results, filter, k, higher_is_better);
        Ok(results)
    }

    /// Filters scored results by metadata and hydrates matching points.
    ///
    /// Shared logic for `search_with_filter` and `search_with_filter_and_opts`
    /// to avoid duplicating the filter-then-hydrate pipeline.
    fn filter_and_hydrate(
        &self,
        index_results: Vec<ScoredResult>,
        filter: &crate::filter::Filter,
        k: usize,
        higher_is_better: bool,
    ) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|sr| {
                let payload = payload_storage.retrieve(sr.id).ok().flatten();
                let matches = match payload.as_ref() {
                    Some(p) => filter.matches(p),
                    None => filter.matches(&serde_json::Value::Null),
                };
                if !matches {
                    return None;
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
            .collect();

        resolve::sort_results_by_metric(&mut results, higher_is_better);
        results.truncate(k);
        tag_vector_component_scores(&mut results);
        results
    }
}

/// Heuristic selectivity estimate based on filter structure.
///
/// Returns a value in `(0, 1]` where 1.0 = no filtering, 0.01 = very selective.
/// Used to compute dynamic over-fetch factor for `search_with_filter`.
fn estimate_filter_selectivity(filter: &crate::filter::Filter) -> f64 {
    estimate_condition_selectivity(&filter.condition)
}

fn estimate_condition_selectivity(cond: &crate::filter::Condition) -> f64 {
    use crate::filter::Condition;
    match cond {
        Condition::Eq { .. } | Condition::IsNull { .. } => 0.1,
        Condition::Gt { .. }
        | Condition::Gte { .. }
        | Condition::Lt { .. }
        | Condition::Lte { .. }
        | Condition::Contains { .. }
        | Condition::Like { .. }
        | Condition::ILike { .. } => 0.3,
        Condition::In { values, .. } => {
            // Reason: values.len() is a small count; f64 precision is sufficient.
            #[allow(clippy::cast_precision_loss)]
            let sel = values.len() as f64 * 0.05;
            sel.min(0.8)
        }
        Condition::Neq { .. } | Condition::IsNotNull { .. } => 0.9,
        Condition::And { conditions } => conditions
            .iter()
            .map(estimate_condition_selectivity)
            .product::<f64>()
            .max(0.01),
        Condition::Or { conditions } => conditions
            .iter()
            .map(estimate_condition_selectivity)
            .sum::<f64>()
            .min(1.0),
        Condition::Not { condition } => (1.0 - estimate_condition_selectivity(condition)).max(0.01),
    }
}
