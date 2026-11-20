//! Strategy-aware fusion for the dense-NEAR + text-MATCH hybrid path (#6).
//!
//! `hybrid_search` (in `text.rs`) is a fixed weighted-RRF over vector and BM25
//! ranks. When a query carries `USING FUSION(strategy = ...)` the requested
//! strategy must actually take effect instead of being silently ignored:
//!
//! - `rrf` / unset -> plain weighted RRF (`hybrid_search`).
//! - `weighted`    -> weighted RRF with `vector_weight`/`graph_weight`
//!   normalized so `graph_weight` influences the BM25 branch.
//! - `maximum` / `average` / `rsf` -> score-level fusion of the raw vector
//!   similarity and BM25 score streams via [`FusionStrategy`].

use crate::collection::search::text::{anchored_depth, rrf_depth, HybridQuery};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::fusion::FusionStrategy;
use crate::point::SearchResult;
use crate::velesql::{FusionClause, FusionStrategyType};

/// A ranked `(id, score)` stream for one fusion branch.
type ScoreStream = Vec<(u64, f32)>;

impl Collection {
    /// Routes a dense-NEAR + text-MATCH hybrid through the requested fusion
    /// strategy (#6). Falls back to plain weighted RRF when no FUSION clause
    /// is present or the strategy is `rrf`. Each branch admits its rows by
    /// `filter` ([`Collection::hybrid_streams`]).
    pub(crate) fn hybrid_search_with_clause(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        fusion: Option<&FusionClause>,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        let query = |depth| HybridQuery {
            vector: vector_query,
            text: text_query,
            depth,
            filter,
            anchors: None,
        };
        self.hybrid_by_clause(
            &query(rrf_depth(k)),
            &query(crate::fusion::dense_sparse_candidate_k(k)),
            k,
            fusion,
        )
    }

    /// Routes an anchored dense-NEAR + text-MATCH hybrid (graph anchor set)
    /// through the requested fusion strategy (#6, anchored path).
    ///
    /// Both branches rank only the anchors `filter` matches
    /// ([`Collection::hybrid_streams`]), whatever the strategy: only the
    /// score/rank fusion of the two streams changes.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub(crate) fn hybrid_search_with_anchors_clause(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        fusion: Option<&FusionClause>,
        anchor_ids: &std::collections::HashSet<u64>,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        let query = HybridQuery {
            vector: vector_query,
            text: text_query,
            depth: anchored_depth(k),
            filter,
            anchors: Some(anchor_ids),
        };
        self.hybrid_by_clause(&query, &query, k, fusion)
    }

    /// Fuses a hybrid's branches by `fusion`: weighted RRF over `ranked`
    /// (no clause, `rrf`, `weighted`), score-level fusion over `scored`
    /// (`maximum`, `average`, `rsf`, `weighted_rrf`) — the two differ only by
    /// how deep each branch ranks.
    fn hybrid_by_clause(
        &self,
        ranked: &HybridQuery<'_>,
        scored: &HybridQuery<'_>,
        k: usize,
        fusion: Option<&FusionClause>,
    ) -> Result<Vec<SearchResult>> {
        let Some(fc) = fusion else {
            return self.hybrid_rrf(ranked, k, None, None);
        };
        match fc.strategy {
            FusionStrategyType::Rrf => {
                self.hybrid_rrf(ranked, k, fc.vector_weight.map(cast_weight), fc.k)
            }
            FusionStrategyType::Weighted => {
                self.hybrid_rrf(ranked, k, Some(normalized_vector_weight(fc)), fc.k)
            }
            FusionStrategyType::Maximum
            | FusionStrategyType::Average
            | FusionStrategyType::Rsf
            | FusionStrategyType::WeightedRrf => {
                self.hybrid_search_score_fused(scored, k, &score_fusion_strategy(fc))
            }
        }
    }

    /// Fuses the raw vector-similarity and BM25 score streams of `query` via
    /// `strategy`. Each stream admits only the rows the query's filter
    /// matches, among its anchors ([`Collection::hybrid_streams`]); the fused
    /// rows are judged by the filter once more as they are hydrated, a
    /// payload having possibly changed since.
    fn hybrid_search_score_fused(
        &self,
        query: &HybridQuery<'_>,
        k: usize,
        strategy: &FusionStrategy,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.query_vector_metric(query.vector)?;
        let (vector_scored, text_stream) = self.hybrid_streams(query, metric)?;
        let vector_stream: ScoreStream = vector_scored.iter().map(|sr| (sr.id, sr.score)).collect();
        let fused = fuse_streams(
            strategy,
            vector_stream,
            metric.score_direction(),
            text_stream,
        )?;
        Ok(match query.filter {
            Some(filter) => self.resolve_fused_matching(&fused, filter, k),
            None => self.resolve_fused_results(&fused, k),
        })
    }

    /// Resolves the first `k` fused points whose payload `filter` matches,
    /// in fused order, reading each candidate's payload before copying its
    /// vector: a point without a payload is judged with a `null` payload, as
    /// on every path.
    fn resolve_fused_matching(
        &self,
        fused: &[(u64, f32)],
        filter: &crate::filter::Filter,
        k: usize,
    ) -> Vec<SearchResult> {
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        crate::collection::search::resolve::hydrate_accepted(
            fused.iter().copied(),
            k,
            crate::collection::expiry::now_unix_secs(),
            crate::collection::search::query::row_universe::RowUniverse::Vectors,
            &*vector_storage,
            &*payload_storage,
            |id, payload| filter.matches_point(id, payload),
        )
    }
}

/// Truncates a `f64` fusion weight to `f32`.
#[allow(clippy::cast_possible_truncation)]
fn cast_weight(w: f64) -> f32 {
    w as f32
}

/// Fuses two `(id, score)` streams via `strategy`, returning an empty list when
/// both inputs are empty (so callers resolve a uniform result set).
///
/// `vector_direction` is the collection metric's polarity: Euclidean/Hamming
/// vector scores are distances and must be declared lower-is-better so
/// score-level strategies do not invert them against the always
/// higher-is-better BM25 stream (#2102).
fn fuse_streams(
    strategy: &FusionStrategy,
    vector_stream: ScoreStream,
    vector_direction: crate::fusion::ScoreDirection,
    text_stream: ScoreStream,
) -> Result<ScoreStream> {
    if vector_stream.is_empty() && text_stream.is_empty() {
        return Ok(Vec::new());
    }
    let directions = [
        vector_direction,
        crate::fusion::ScoreDirection::HigherIsBetter,
    ];
    strategy
        .fuse_with_directions(vec![vector_stream, text_stream], &directions)
        .map_err(Error::from)
}

/// Normalizes `vector_weight` against `graph_weight` so the BM25 branch weight
/// (`1 - vector_weight` inside `hybrid_search`) reflects `graph_weight` (#6).
fn normalized_vector_weight(fc: &FusionClause) -> f32 {
    let vw = fc.vector_weight.map_or(0.5, cast_weight);
    let gw = fc.graph_weight.map_or(0.5, cast_weight);
    let total = vw + gw;
    if total <= 0.0 {
        return 0.5;
    }
    vw / total
}

/// Builds the score-level `FusionStrategy` for a Maximum/Average/Rsf clause.
fn score_fusion_strategy(fc: &FusionClause) -> FusionStrategy {
    match fc.strategy {
        FusionStrategyType::Maximum => FusionStrategy::Maximum,
        FusionStrategyType::Rsf => {
            let dw = fc.dense_weight.unwrap_or(0.5);
            let sw = fc.sparse_weight.unwrap_or(0.5);
            // Weights are validate-time checked; the fallback only guards the
            // unreachable invalid case rather than silently degrading.
            FusionStrategy::relative_score(dw, sw).unwrap_or(FusionStrategy::RelativeScore {
                dense_weight: 0.5,
                sparse_weight: 0.5,
            })
        }
        FusionStrategyType::WeightedRrf => {
            let dw = fc.dense_weight.unwrap_or(0.5);
            let sw = fc.sparse_weight.unwrap_or(0.5);
            #[allow(clippy::cast_precision_loss)]
            let k = fc.k.unwrap_or(60) as f32;
            FusionStrategy::weighted_rrf(vec![dw, sw], k)
                .unwrap_or_else(|_| FusionStrategy::rrf_default())
        }
        _ => FusionStrategy::Average,
    }
}
