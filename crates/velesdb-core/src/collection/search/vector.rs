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

    /// Merges HNSW results with the delta buffer (if active).
    ///
    /// When the delta buffer is inactive (no rebuild in progress), this is
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
        let tuples: Vec<(u64, f32)> = results.into_iter().map(Into::into).collect();
        crate::collection::streaming::merge_with_delta(tuples, &self.delta_buffer, query, k, metric)
            .into_iter()
            .map(ScoredResult::from)
            .collect()
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

        Ok(resolve::resolve_scored_results(
            &index_results,
            &*vector_storage,
            &*payload_storage,
        ))
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
            129..=256 => crate::SearchQuality::Accurate,
            _ => crate::SearchQuality::Perfect,
        };

        let metric = self.config.read().metric;
        let index_results = self.index.search_with_quality(query, k, quality);
        let index_results = self.merge_delta(index_results, query, k, metric);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        Ok(resolve::resolve_scored_results(
            &index_results,
            &*vector_storage,
            &*payload_storage,
        ))
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

        let candidates_k = k.saturating_mul(4).max(k + 10);
        let index_results = self.search_ids_with_adc_if_pq(query, candidates_k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|sr| {
                let vector = vector_storage.retrieve(sr.id).ok().flatten()?;
                let payload = payload_storage.retrieve(sr.id).ok().flatten();
                let matches = match payload.as_ref() {
                    Some(p) => filter.matches(p),
                    None => filter.matches(&serde_json::Value::Null),
                };
                if !matches {
                    return None;
                }
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
        Ok(results)
    }
}
