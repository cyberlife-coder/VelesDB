//! Text and hybrid search methods for Collection.

use super::resolve;
use super::OrderedFloat;
use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;
use std::collections::HashSet;

/// Anchor-restricted hybrid streams: scored vector branch + `(id, score)` BM25
/// branch, both confined to the anchor set (shared by the RRF and score-level
/// anchored hybrid paths).
type AnchoredHybridStreams = (Vec<crate::scored_result::ScoredResult>, Vec<(u64, f32)>);

/// Resolves `(weight, text_weight, rrf_constant)` from optional caller inputs.
///
/// `vector_weight` defaults to 0.5; `rrf_k` defaults to 60.
#[allow(clippy::cast_precision_loss)]
fn resolve_rrf_params(vector_weight: Option<f32>, rrf_k: Option<u32>) -> (f32, f32, f32) {
    let w = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
    // u32→f32: RRF k is typically 1–1000, lossless below 2^24.
    let k = rrf_k.unwrap_or(60).max(1) as f32;
    (w, 1.0 - w, k)
}

/// Validates the query vector dimension and resolves RRF parameters.
///
/// Returns `(metric, weight, text_weight, rrf_constant)`.
fn validated_hybrid_params(
    config: &parking_lot::RwLockReadGuard<'_, crate::collection::types::CollectionConfig>,
    vector_query: &[f32],
    vector_weight: Option<f32>,
    rrf_k: Option<u32>,
) -> Result<(crate::DistanceMetric, f32, f32, f32)> {
    validate_dimension_match(config.dimension, vector_query.len())?;
    let metric = config.metric;
    let (weight, text_weight, rrf_constant) = resolve_rrf_params(vector_weight, rrf_k);
    Ok((metric, weight, text_weight, rrf_constant))
}

/// Fetches vector + BM25 candidates and fuses them with weighted RRF.
///
/// Returns `(fused_scores, component_map)`. Both branches use `overfetch_k`
/// candidates to ensure the top-k fused results are drawn from a deep enough
/// candidate pool.
#[allow(clippy::too_many_arguments)] // All args come from validated_hybrid_params + overfetch_k.
fn compute_hybrid_scored(
    collection: &Collection,
    vector_query: &[f32],
    text_query: &str,
    overfetch_k: usize,
    metric: crate::DistanceMetric,
    weight: f32,
    text_weight: f32,
    rrf_constant: f32,
) -> (
    rustc_hash::FxHashMap<u64, f32>,
    rustc_hash::FxHashMap<u64, (f32, f32)>,
) {
    use crate::index::VectorIndex;
    let raw = collection.index.search(vector_query, overfetch_k);
    let vec_res = collection.merge_delta(raw, vector_query, overfetch_k, metric);
    let text_res = collection.text_index.search(text_query, overfetch_k);
    Collection::compute_rrf_scores_with_components(
        &vec_res,
        &text_res,
        weight,
        text_weight,
        rrf_constant,
    )
}

/// Attaches RRF component scores to a `SearchResult` from the component map.
fn attach_rrf_components(
    result: &mut SearchResult,
    component_map: &rustc_hash::FxHashMap<u64, (f32, f32)>,
) {
    if let Some(&(vec_score, bm25_score)) = component_map.get(&result.point.id) {
        result.component_scores = Some(smallvec::smallvec![
            ("vector_score", vec_score),
            ("bm25_score", bm25_score),
        ]);
    }
}

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
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn text_search(&self, query: &str, k: usize) -> Result<Vec<SearchResult>> {
        let bm25_results = self.text_index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results = resolve::resolve_id_score_pairs(
            &bm25_results,
            bm25_results.len(),
            &*vector_storage,
            &*payload_storage,
        );
        // Tag each result with its BM25 component score.
        for result in &mut results {
            result.component_scores = Some(smallvec::smallvec![("bm25_score", result.score),]);
        }
        Ok(results)
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
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
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
        let now_secs = now_unix_secs();

        Ok(bm25_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();
                if is_payload_expired(payload.as_ref(), now_secs) {
                    return None;
                }

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

                Some(SearchResult::with_component_scores(
                    point,
                    score,
                    smallvec::smallvec![("bm25_score", score)],
                ))
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
    /// * `rrf_k` - RRF constant (default 60). Lower values amplify rank differences.
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
        rrf_k: Option<u32>,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();
        let (metric, weight, text_weight, rrf_constant) =
            validated_hybrid_params(&config, vector_query, vector_weight, rrf_k)?;
        drop(config);

        let (fused_scores, component_map) = compute_hybrid_scored(
            self,
            vector_query,
            text_query,
            k * 2,
            metric,
            weight,
            text_weight,
            rrf_constant,
        );

        let scored_ids = Self::top_k_from_scores(fused_scores, k);
        Ok(self.resolve_scored_ids_with_components(&scored_ids, &component_map))
    }

    /// Computes RRF fused scores and per-component score breakdowns.
    ///
    /// The `rrf_k` parameter controls the RRF constant (default 60.0). Lower
    /// values amplify rank differences; higher values smooth them out.
    ///
    /// Returns `(fused_scores, component_map)` where `component_map` maps each
    /// point ID to its individual `(vector_rrf, bm25_rrf)` contributions.
    // EPIC-040: FusionStrategy::WeightedRRF now provides the same weighted,
    // 0-based entry point (weight/(rank+k)). This method keeps its one-pass
    // implementation to build the fused-score map and per-component breakdown
    // simultaneously, avoiding a second iteration over the branches for the
    // score-explanation path.
    #[allow(clippy::cast_precision_loss)]
    fn compute_rrf_scores_with_components(
        vector_results: &[crate::scored_result::ScoredResult],
        text_results: &[(u64, f32)],
        vector_weight: f32,
        text_weight: f32,
        rrf_k: f32,
    ) -> (
        rustc_hash::FxHashMap<u64, f32>,
        rustc_hash::FxHashMap<u64, (f32, f32)>,
    ) {
        let cap = vector_results.len() + text_results.len();
        let mut fused: rustc_hash::FxHashMap<u64, f32> =
            rustc_hash::FxHashMap::with_capacity_and_hasher(cap, rustc_hash::FxBuildHasher);
        let mut components: rustc_hash::FxHashMap<u64, (f32, f32)> =
            rustc_hash::FxHashMap::with_capacity_and_hasher(cap, rustc_hash::FxBuildHasher);

        for (rank, sr) in vector_results.iter().enumerate() {
            let contribution = vector_weight / (rank as f32 + rrf_k);
            *fused.entry(sr.id).or_insert(0.0) += contribution;
            components.entry(sr.id).or_insert((0.0, 0.0)).0 += contribution;
        }
        for (rank, (id, _)) in text_results.iter().enumerate() {
            let contribution = text_weight / (rank as f32 + rrf_k);
            *fused.entry(*id).or_insert(0.0) += contribution;
            components.entry(*id).or_insert((0.0, 0.0)).1 += contribution;
        }
        (fused, components)
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
        scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        scored
    }

    /// Resolves scored IDs to `SearchResult` with per-component score breakdown.
    fn resolve_scored_ids_with_components(
        &self,
        scored_ids: &[(u64, f32)],
        component_map: &rustc_hash::FxHashMap<u64, (f32, f32)>,
    ) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();
        let now_secs = now_unix_secs();

        scored_ids
            .iter()
            .filter_map(|&(id, score)| {
                let mut result = resolve::hydrate_point(
                    id,
                    score,
                    now_secs,
                    &*vector_storage,
                    &*payload_storage,
                )?;
                attach_rrf_components(&mut result, component_map);
                Some(result)
            })
            .collect()
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
    /// * `rrf_k` - RRF constant (default 60). Lower values amplify rank differences.
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
        rrf_k: Option<u32>,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();
        let (metric, weight, text_weight, rrf_constant) =
            validated_hybrid_params(&config, vector_query, vector_weight, rrf_k)?;
        drop(config);

        let candidates_k = k.saturating_mul(4).max(k + 10);
        let (fused_scores, component_map) = compute_hybrid_scored(
            self,
            vector_query,
            text_query,
            candidates_k,
            metric,
            weight,
            text_weight,
            rrf_constant,
        );

        let mut scored_ids: Vec<_> = fused_scores.into_iter().collect();
        scored_ids.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));

        Ok(
            self.resolve_scored_ids_filtered_with_components(
                &scored_ids,
                filter,
                k,
                &component_map,
            ),
        )
    }

    /// Hybrid search restricted to `anchor_ids` in both vector and BM25 branches.
    ///
    /// Used when a graph MATCH predicate AND-requires anchor membership: RRF
    /// fusion only considers points in the anchor set, so a relevant anchor
    /// outside the global top-K is still surfaced.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub(crate) fn hybrid_search_with_anchors(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
        rrf_k: Option<u32>,
        anchor_ids: &HashSet<u64>,
    ) -> Result<Vec<SearchResult>> {
        let (weight, text_weight, rrf_constant) = resolve_rrf_params(vector_weight, rrf_k);
        let overfetch_k = k.saturating_mul(4).max(k + 10);
        let (vector_scored, text_results) =
            self.anchored_hybrid_streams(vector_query, text_query, anchor_ids, overfetch_k)?;

        let (fused_scores, component_map) = Self::compute_rrf_scores_with_components(
            &vector_scored,
            &text_results,
            weight,
            text_weight,
            rrf_constant,
        );

        let scored_ids = Self::top_k_from_scores(fused_scores, k);
        Ok(self.resolve_scored_ids_with_components(&scored_ids, &component_map))
    }

    /// Builds the anchor-restricted vector-similarity and BM25 score streams
    /// shared by the RRF and score-level anchored hybrid paths.
    ///
    /// Both branches are confined to `anchor_ids`, so the candidate set is
    /// identical regardless of the downstream fusion strategy.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub(crate) fn anchored_hybrid_streams(
        &self,
        vector_query: &[f32],
        text_query: &str,
        anchor_ids: &HashSet<u64>,
        overfetch_k: usize,
    ) -> Result<AnchoredHybridStreams> {
        let config = self.config.read();
        validate_dimension_match(config.dimension, vector_query.len())?;
        drop(config);

        // Vector branch: restrict retrieval to anchor set.
        let anchor_search =
            self.search_near_with_anchor_ids(vector_query, anchor_ids, None, overfetch_k)?;
        let vector_scored: Vec<crate::scored_result::ScoredResult> = anchor_search
            .into_iter()
            .map(|r| crate::scored_result::ScoredResult::new(r.point.id, r.score))
            .collect();

        // BM25 branch: over-fetch then restrict to anchor set.
        let bm25_all = self.text_index.search(text_query, overfetch_k);
        let text_results: Vec<(u64, f32)> = bm25_all
            .into_iter()
            .filter(|(id, _)| anchor_ids.contains(id))
            .collect();

        Ok((vector_scored, text_results))
    }

    /// Resolves scored IDs with filter and optional per-component score breakdown.
    fn resolve_scored_ids_filtered_with_components(
        &self,
        scored_ids: &[(u64, f32)],
        filter: &crate::filter::Filter,
        k: usize,
        component_map: &rustc_hash::FxHashMap<u64, (f32, f32)>,
    ) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();
        let now_secs = now_unix_secs();

        scored_ids
            .iter()
            .filter_map(|&(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();
                if is_payload_expired(payload.as_ref(), now_secs) {
                    return None;
                }
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
                let mut result = SearchResult::new(point, score);
                attach_rrf_components(&mut result, component_map);
                Some(result)
            })
            .take(k)
            .collect()
    }
}
