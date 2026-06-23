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
    /// is present or the strategy is `rrf`.
    pub(crate) fn hybrid_search_with_clause(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        fusion: Option<&FusionClause>,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        let Some(fc) = fusion else {
            return self.hybrid_search_default(vector_query, text_query, k, None, filter);
        };
        match fc.strategy {
            FusionStrategyType::Rrf => {
                let vw = fc.vector_weight.map(cast_weight);
                self.hybrid_search_default(vector_query, text_query, k, vw, filter)
            }
            FusionStrategyType::Weighted => {
                let vw = normalized_vector_weight(fc);
                self.hybrid_search_default(vector_query, text_query, k, Some(vw), filter)
            }
            FusionStrategyType::Maximum | FusionStrategyType::Average | FusionStrategyType::Rsf => {
                let strategy = score_fusion_strategy(fc);
                self.hybrid_search_score_fused(vector_query, text_query, k, &strategy, filter)
            }
        }
    }

    /// Routes an anchored dense-NEAR + text-MATCH hybrid (graph anchor set)
    /// through the requested fusion strategy (#6, anchored path).
    ///
    /// The anchor-restricted candidate set is identical for every strategy
    /// (both branches are confined to `anchor_ids`); only the score/rank
    /// fusion of the two streams changes. Falls back to the existing
    /// anchored RRF when no FUSION clause is present or the strategy is `rrf`.
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
    ) -> Result<Vec<SearchResult>> {
        let Some(fc) = fusion else {
            return self.hybrid_search_with_anchors(
                vector_query,
                text_query,
                k,
                None,
                None,
                anchor_ids,
            );
        };
        match fc.strategy {
            FusionStrategyType::Rrf => {
                let vw = fc.vector_weight.map(cast_weight);
                self.hybrid_search_with_anchors(vector_query, text_query, k, vw, fc.k, anchor_ids)
            }
            FusionStrategyType::Weighted => {
                let vw = normalized_vector_weight(fc);
                self.hybrid_search_with_anchors(
                    vector_query,
                    text_query,
                    k,
                    Some(vw),
                    fc.k,
                    anchor_ids,
                )
            }
            FusionStrategyType::Maximum | FusionStrategyType::Average | FusionStrategyType::Rsf => {
                let strategy = score_fusion_strategy(fc);
                self.anchored_hybrid_score_fused(vector_query, text_query, k, &strategy, anchor_ids)
            }
        }
    }

    /// Score-level fusion of the anchor-restricted vector and BM25 streams.
    fn anchored_hybrid_score_fused(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        strategy: &FusionStrategy,
        anchor_ids: &std::collections::HashSet<u64>,
    ) -> Result<Vec<SearchResult>> {
        let overfetch_k = k.saturating_mul(4).max(k + 10);
        let (vector_scored, text_stream) =
            self.anchored_hybrid_streams(vector_query, text_query, anchor_ids, overfetch_k)?;
        let vector_stream: ScoreStream = vector_scored.iter().map(|sr| (sr.id, sr.score)).collect();
        if vector_stream.is_empty() && text_stream.is_empty() {
            return Ok(Vec::new());
        }
        let fused = strategy
            .fuse(vec![vector_stream, text_stream])
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;
        Ok(self.resolve_fused_results(&fused, k))
    }

    /// Plain weighted-RRF hybrid, honoring an optional metadata filter.
    fn hybrid_search_default(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        match filter {
            Some(f) => {
                self.hybrid_search_with_filter(vector_query, text_query, k, vector_weight, f, None)
            }
            None => self.hybrid_search(vector_query, text_query, k, vector_weight, None),
        }
    }

    /// Fuses the raw vector-similarity and BM25 score streams via `strategy`,
    /// then applies the optional metadata filter post-fusion.
    fn hybrid_search_score_fused(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        strategy: &FusionStrategy,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        let candidate_k = k.saturating_mul(2).max(k + 10);
        let (vector_stream, text_stream) =
            self.hybrid_score_streams(vector_query, text_query, candidate_k)?;
        if vector_stream.is_empty() && text_stream.is_empty() {
            return Ok(Vec::new());
        }
        let fused = strategy
            .fuse(vec![vector_stream, text_stream])
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;
        let resolved = self.resolve_fused_results(&fused, fused.len());
        Ok(apply_post_filter(resolved, filter, k))
    }

    /// Builds the `(id, score)` vector-similarity and BM25 streams used by
    /// score-level fusion.
    fn hybrid_score_streams(
        &self,
        vector_query: &[f32],
        text_query: &str,
        candidate_k: usize,
    ) -> Result<(ScoreStream, ScoreStream)> {
        use crate::index::VectorIndex;
        let metric = {
            let config = self.config.read();
            crate::validation::validate_dimension_match(config.dimension, vector_query.len())?;
            config.metric
        };
        let raw = self.index.search(vector_query, candidate_k);
        let vec_res = self.merge_delta(raw, vector_query, candidate_k, metric);
        let vector_stream: Vec<(u64, f32)> = vec_res.iter().map(|sr| (sr.id, sr.score)).collect();
        let text_stream = self.text_index.search(text_query, candidate_k);
        Ok((vector_stream, text_stream))
    }
}

/// Truncates a `f64` fusion weight to `f32`.
#[allow(clippy::cast_possible_truncation)]
fn cast_weight(w: f64) -> f32 {
    w as f32
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
        _ => FusionStrategy::Average,
    }
}

/// Applies an optional metadata filter to fused results, truncating to `k`.
fn apply_post_filter(
    results: Vec<SearchResult>,
    filter: Option<&crate::filter::Filter>,
    k: usize,
) -> Vec<SearchResult> {
    let Some(filter) = filter else {
        return results.into_iter().take(k).collect();
    };
    results
        .into_iter()
        .filter(|r| match r.point.payload.as_ref() {
            Some(p) => filter.matches(p),
            None => false,
        })
        .take(k)
        .collect()
}
