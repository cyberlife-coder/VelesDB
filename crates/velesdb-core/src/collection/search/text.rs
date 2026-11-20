//! Text and hybrid search methods for Collection.

use super::pending::PendingRestriction;
use super::resolve;
use crate::collection::expiry::now_unix_secs;
use crate::collection::search::query::row_universe::{RowScope, RowUniverse};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use std::collections::HashSet;

/// The two ranked branches of a hybrid search: the scored vector branch and
/// the `(id, score)` BM25 branch ([`Collection::hybrid_streams`]).
type HybridStreams = (Vec<crate::scored_result::ScoredResult>, Vec<(u64, f32)>);

/// Resolves `(weight, text_weight, rrf_constant)` from optional caller inputs.
///
/// `vector_weight` is resolved by
/// [`crate::validation::resolve_hybrid_vector_weight`] (default 0.5, a
/// non-finite weight refused); `rrf_k` defaults to 60.
#[allow(clippy::cast_precision_loss)]
fn resolve_rrf_params(vector_weight: Option<f32>, rrf_k: Option<u32>) -> Result<(f32, f32, f32)> {
    let w = crate::validation::resolve_hybrid_vector_weight(vector_weight)?;
    // u32→f32: RRF k is typically 1–1000, lossless below 2^24.
    let k = rrf_k.unwrap_or(60).max(1) as f32;
    Ok((w, 1.0 - w, k))
}

/// How many of each branch's best rows a weighted-RRF hybrid search fuses
/// for `k` results: twice `k`. A filter does not deepen it — each branch
/// admits only the rows the filter matches ([`HybridQuery::filter`]).
pub(super) const fn rrf_depth(k: usize) -> usize {
    k.saturating_mul(2)
}

/// How many of each branch's best anchors an anchored hybrid search fuses
/// for `k` results: four times `k`, at least `k + 10`.
pub(super) fn anchored_depth(k: usize) -> usize {
    k.saturating_mul(4).max(k.saturating_add(10))
}

/// What a hybrid search fuses: a vector and a text query, each branch's
/// best `depth` rows, the payload filter both branches admit their rows by,
/// and the graph anchors they are confined to, if any. A row is fused when
/// it is among the best `depth` rows of a branch that hold both; the text
/// branch walks every BM25 candidate for them (`Collection::ranked_rows`),
/// and the vector branch is the filtered or anchored vector search, whose
/// own candidate bounds apply ([`Collection::search_with_filter`],
/// [`Collection::search_near_within_anchors`]).
pub(super) struct HybridQuery<'a> {
    pub(super) vector: &'a [f32],
    pub(super) text: &'a str,
    pub(super) depth: usize,
    pub(super) filter: Option<&'a crate::filter::Filter>,
    pub(super) anchors: Option<&'a HashSet<u64>>,
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
    /// A hit is any row of the collection whose payload text matches: a
    /// metadata row and a graph node without an embedding included, read with
    /// an empty `vector` as SELECT reads them ([`Self::text_hits`]).
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
        Ok(self.text_hits(query, k, None))
    }

    /// Performs full-text search with metadata filtering: the best `k` hits
    /// whose payload `filter` matches, however far below the best of the
    /// collection they rank — the filter is part of the walk's admission
    /// ([`Self::text_hits`]).
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
        Ok(self.text_hits(query, k, Some(filter)))
    }

    /// The best `k` BM25 hits for `query` whose payload `filter`, when
    /// given, matches (a point without payload is tested with a `null`
    /// payload, as on every path), in BM25 order, each tagged with its
    /// `bm25_score`. The hits are the rows of this collection
    /// ([`RowScope::All`]): an indexed document of an id that is no row —
    /// what an earlier version let a vector collection store without a
    /// vector — never takes a row's place among the `k`, and neither does a
    /// row the filter refuses ([`Collection::ranked_hits`]).
    ///
    /// LOCK ORDER: config(1), released; the BM25 index's own locks,
    /// released; secondary_indexes(6) for the prefilter, released; then
    /// vector_storage(2) before payload_storage(3), per batch of the walk.
    fn text_hits(
        &self,
        query: &str,
        k: usize,
        filter: Option<&crate::filter::Filter>,
    ) -> Vec<SearchResult> {
        let rows = RowScope::All.universe(self.row_universe());
        let ranked = self.ranked_within_prefilter(self.storage.text_index.score_all(query), filter);
        let mut results = self.ranked_hits(
            ranked,
            k,
            rows,
            |_| true,
            |id, payload| filter.is_none_or(|filter| filter.matches_point(id, payload)),
        );
        for result in &mut results {
            result.component_scores = Some(smallvec::smallvec![("bm25_score", result.score)]);
        }
        results
    }

    /// The best `k` BM25 hits for `query` among the ids that are rows of
    /// `universe`, best first: an indexed document of any other id never
    /// takes a row's place among the `k` ([`super::ranked::Ranked`]). A vector collection
    /// may hold such documents — the payloads an earlier version let
    /// `upsert_metadata` store without a vector.
    ///
    /// LOCK ORDER: the BM25 index's own locks, released; then
    /// vector_storage(2) before payload_storage(3), both read, per batch of
    /// the walk ([`Collection::ranked_rows`]).
    pub(crate) fn bm25_rows(
        &self,
        query: &str,
        k: usize,
        universe: RowUniverse,
    ) -> Vec<(u64, f32)> {
        self.bm25_rows_where(query, k, universe, |_| true, None)
    }

    /// [`Self::bm25_rows`] over the rows `within` also takes and, given a
    /// `filter`, whose payload it matches ([`Collection::ranked_rows`]).
    pub(crate) fn bm25_rows_where(
        &self,
        query: &str,
        k: usize,
        universe: RowUniverse,
        within: impl Fn(u64) -> bool,
        filter: Option<&crate::filter::Filter>,
    ) -> Vec<(u64, f32)> {
        let ranked = self.ranked_within_prefilter(self.storage.text_index.score_all(query), filter);
        self.ranked_rows(ranked, k, universe, within, filter)
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
    /// Returns an error if the query vector dimension doesn't match, or
    /// [`crate::Error::Config`] if `vector_weight` is not finite.
    pub fn hybrid_search(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
        rrf_k: Option<u32>,
    ) -> Result<Vec<SearchResult>> {
        let query = HybridQuery {
            vector: vector_query,
            text: text_query,
            depth: rrf_depth(k),
            filter: None,
            anchors: None,
        };
        self.hybrid_rrf(&query, k, vector_weight, rrf_k)
    }

    /// The weighted-RRF fusion of `query`'s two branches, its best `k`
    /// rows — those whose payload its filter matches, read again as they
    /// are hydrated — each with its component scores.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match, or
    /// [`crate::Error::Config`] if `vector_weight` is not finite.
    pub(super) fn hybrid_rrf(
        &self,
        query: &HybridQuery<'_>,
        k: usize,
        vector_weight: Option<f32>,
        rrf_k: Option<u32>,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.query_vector_metric(query.vector)?;
        let (weight, text_weight, rrf_constant) = resolve_rrf_params(vector_weight, rrf_k)?;
        let (vector_scored, text_scored) = self.hybrid_streams(query, metric)?;
        let (fused_scores, component_map) = Self::compute_rrf_scores_with_components(
            &vector_scored,
            &text_scored,
            weight,
            text_weight,
            rrf_constant,
        );
        let mut scored_ids: Vec<_> = fused_scores.into_iter().collect();
        crate::fusion::sort_fused_results(&mut scored_ids);
        // A point without a payload is judged with a `null` payload, as on
        // every path.
        Ok(self.resolve_scored_ids_with_components(
            &scored_ids,
            &component_map,
            |id, payload| query.filter.is_none_or(|f| f.matches_point(id, payload)),
            k,
        ))
    }

    /// The metric of this collection, once `vector` is checked against it.
    ///
    /// # Errors
    ///
    /// Returns an error if `vector`'s dimension does not match.
    ///
    /// LOCK ORDER: config(1) alone.
    pub(super) fn query_vector_metric(&self, vector: &[f32]) -> Result<crate::DistanceMetric> {
        let config = self.storage.config.read();
        config.query_vector_target().check_len(vector.len())?;
        Ok(config.metric)
    }

    /// The two branches of a hybrid search, each `query.depth` deep: the
    /// nearest vectors and the best BM25 hits among the vector rows. Each
    /// branch admits only the rows the filter matches, among the anchors
    /// when there are — the anchored or filtered vector search
    /// ([`Self::search_near_within_anchors`], [`Self::search_with_filter`])
    /// and the BM25 walk admitting by both ([`Self::bm25_rows_where`]) — so
    /// the filter and the anchors decide which rows are fused, not which of
    /// the fused rows are kept. `metric` is the collection's, checked against
    /// the vector ([`Self::query_vector_metric`]).
    ///
    /// # Errors
    ///
    /// Returns an error if the vector search fails.
    pub(super) fn hybrid_streams(
        &self,
        query: &HybridQuery<'_>,
        metric: crate::DistanceMetric,
    ) -> Result<HybridStreams> {
        let as_scored = |results: Vec<SearchResult>| {
            results
                .into_iter()
                .map(|r| crate::scored_result::ScoredResult::new(r.point.id, r.score))
                .collect()
        };
        let vector_scored = match (query.anchors, query.filter) {
            (Some(anchors), filter) => as_scored(self.search_near_within_anchors(
                query.vector,
                anchors,
                filter,
                query.depth,
            )?),
            (None, Some(filter)) => {
                as_scored(self.search_with_filter(query.vector, query.depth, filter)?)
            }
            // `search_with_quality`, not `search`: the latter pins
            // `Balanced` and so ignores the configured `[search]` quality
            // (#2430). The nearest live rows: an expired one takes no place.
            (None, None) => as_scored(self.nearest_live(
                &super::live::Nearest::rows(query.vector, metric, query.depth),
                |asked| {
                    self.search_merging_pending(
                        query.vector,
                        asked,
                        metric,
                        PendingRestriction::NONE,
                        |n| {
                            self.storage.index.search_with_quality(
                                query.vector,
                                n,
                                self.runtime_search_quality(),
                            )
                        },
                    )
                },
            )?),
        };
        let text_scored = self.bm25_rows_where(
            query.text,
            query.depth,
            RowUniverse::Vectors,
            |id| query.anchors.is_none_or(|anchors| anchors.contains(&id)),
            query.filter,
        );
        Ok((vector_scored, text_scored))
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

    /// Resolves scored IDs to `SearchResult` with per-component score
    /// breakdown, keeping the first `limit` points `accept` takes (it sees
    /// the point's id and payload), in `scored_ids` order; a rejected
    /// point's vector is never copied ([`resolve::hydrate_accepted`]).
    fn resolve_scored_ids_with_components(
        &self,
        scored_ids: &[(u64, f32)],
        component_map: &rustc_hash::FxHashMap<u64, (f32, f32)>,
        accept: impl Fn(u64, Option<&serde_json::Value>) -> bool,
        limit: usize,
    ) -> Vec<SearchResult> {
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let mut results = resolve::hydrate_accepted(
            scored_ids.iter().copied(),
            limit,
            now_unix_secs(),
            RowUniverse::Vectors,
            &*vector_storage,
            &*payload_storage,
            accept,
        );
        drop(payload_storage);
        drop(vector_storage);
        for result in &mut results {
            attach_rrf_components(result, component_map);
        }
        results
    }

    /// Performs hybrid search (vector + text) with metadata filtering.
    ///
    /// Uses Reciprocal Rank Fusion (RRF) to combine the best rows of both
    /// searches that the filter matches: each branch admits its rows by the
    /// filter ([`Self::hybrid_streams`]). The text branch walks every BM25
    /// candidate, so a matching row ranked below every other row of the
    /// collection is still fused; the vector branch is the filtered vector
    /// search, which ranks the matching rows among the candidates its
    /// oversampling asks the index for (at most 10 000 beyond `k`), or all of
    /// them when the filter's prefilter bitmap restricts the index walk.
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
    /// Returns an error if the query vector dimension doesn't match, or
    /// [`crate::Error::Config`] if `vector_weight` is not finite.
    pub fn hybrid_search_with_filter(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
        filter: &crate::filter::Filter,
        rrf_k: Option<u32>,
    ) -> Result<Vec<SearchResult>> {
        let query = HybridQuery {
            vector: vector_query,
            text: text_query,
            depth: rrf_depth(k),
            filter: Some(filter),
            anchors: None,
        };
        self.hybrid_rrf(&query, k, vector_weight, rrf_k)
    }
}
