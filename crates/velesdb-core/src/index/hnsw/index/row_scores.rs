//! Scores of individual mapped rows when a SQL order reads an entire ranking.

use super::HnswIndex;
use crate::scored_result::ScoredResult;
use crate::SearchQuality;

/// The native scorer that gives a NEAR result its primary score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndexRowScores {
    Graph,
    Arena,
    Bitmap,
    Rerank { device: bool },
}

impl HnswIndex {
    /// Whether a native prefix can be ranked by a device reduction instead
    /// of the original row's host SIMD calculation. Planning does not need
    /// to allocate GPU resources or assume that a dispatch will succeed.
    #[cfg(feature = "gpu")]
    pub(crate) fn device_scoring_eligible(
        &self,
        quality: SearchQuality,
        rerank: Option<bool>,
    ) -> bool {
        use crate::DistanceMetric;
        if matches!(
            self.metric,
            DistanceMetric::Hamming | DistanceMetric::Jaccard
        ) {
            return false;
        }
        let device_rerank =
            crate::gpu::GpuAccelerator::should_rerank_gpu(self.len(), self.dimension);
        if rerank == Some(true) {
            return device_rerank;
        }
        if self.searches_arena_exhaustively(quality, rerank) {
            return false;
        }
        let graph = self.inner.read().gpu_traversal_eligible(self.dimension);
        let automatic = rerank.is_none()
            && self.enable_vector_storage
            && matches!(
                quality,
                SearchQuality::Fast
                    | SearchQuality::Balanced
                    | SearchQuality::Accurate
                    | SearchQuality::Custom(_)
            );
        graph || (automatic && device_rerank)
    }

    /// Only a supported metric on the eligible full-precision graph can
    /// expose device distances in the final, non-reranked hits.
    #[cfg(feature = "gpu")]
    pub(crate) fn graph_scores_may_use_device(&self) -> bool {
        matches!(
            self.metric,
            crate::DistanceMetric::Cosine
                | crate::DistanceMetric::Euclidean
                | crate::DistanceMetric::DotProduct
        ) && self.inner.read().gpu_traversal_eligible(self.dimension)
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn has_graph_score_device(&self) -> bool {
        self.graph_scores_may_use_device() && crate::gpu::GpuTraversalContext::global().is_some()
    }

    pub(crate) fn bitmap_row_scores(&self, quality: SearchQuality) -> IndexRowScores {
        if quality == SearchQuality::Perfect || self.exact_small_index() {
            IndexRowScores::Bitmap
        } else {
            IndexRowScores::Graph
        }
    }

    pub(crate) fn searches_arena_exhaustively(
        &self,
        quality: SearchQuality,
        rerank: Option<bool>,
    ) -> bool {
        rerank.is_none()
            && (self.exact_small_index()
                || (quality == SearchQuality::Perfect && self.enable_vector_storage))
    }

    /// At full depth, the automatic two-stage rerank cannot fire:
    /// `should_two_stage_rerank` requires more than twice the requested rows.
    pub(crate) fn full_depth_row_scores(
        &self,
        quality: SearchQuality,
        rerank: Option<bool>,
    ) -> IndexRowScores {
        match rerank {
            Some(true) => self.rerank_row_scores(self.len()),
            None if self.searches_arena_exhaustively(quality, None) => IndexRowScores::Arena,
            Some(false) | None => IndexRowScores::Graph,
        }
    }

    fn rerank_row_scores(&self, candidates: usize) -> IndexRowScores {
        #[cfg(feature = "gpu")]
        let device = crate::gpu::GpuAccelerator::should_rerank_gpu(candidates, self.dimension);
        #[cfg(not(feature = "gpu"))]
        let device = {
            let _ = (self, candidates);
            false
        };
        IndexRowScores::Rerank { device }
    }

    /// Scores only the mapped ids in `candidates`, preserving their order.
    /// A missing mapping keeps the caller's current-storage score.
    pub(crate) fn score_admitted_rows(
        &self,
        query: &[f32],
        candidates: &mut [ScoredResult],
        source: IndexRowScores,
    ) {
        if let IndexRowScores::Rerank { device } = source {
            let rescored = self.rerank_admitted_rows(query, candidates, device);
            apply_reranked_scores(candidates, &rescored);
            return;
        }
        let inner = self.inner.read();
        let graph = inner.scoring_graph();
        let mut score_rows = |q: &[f32]| {
            graph.with_vectors_read(|vectors| {
                for row in candidates.iter_mut() {
                    let Some(v) = self.mappings.get_idx(row.id).and_then(|i| vectors.get(i)) else {
                        continue;
                    };
                    row.score = match source {
                        IndexRowScores::Graph => {
                            graph.transform_score(graph.compute_distance(q, v))
                        }
                        IndexRowScores::Bitmap => self.score(q, v),
                        IndexRowScores::Arena | IndexRowScores::Rerank { .. } => {
                            self.compute_distance(q, v)
                        }
                    };
                }
            });
        };
        if source == IndexRowScores::Graph {
            graph.with_prepared_query(query, score_rows);
        } else {
            score_rows(query);
        }
        drop(inner);
    }

    /// The source is selected by the depth of the native candidate pool.
    /// Filtering down to one row in this batch cannot change that choice.
    fn rerank_admitted_rows(
        &self,
        query: &[f32],
        rows: &[ScoredResult],
        device: bool,
    ) -> Vec<ScoredResult> {
        #[cfg(feature = "gpu")]
        if device {
            if let Some(scores) = self.rerank_candidates_gpu(query, rows) {
                return scores;
            }
        }
        #[cfg(not(feature = "gpu"))]
        let _ = device;
        self.rerank_candidates_simd(query, rows)
    }
}

/// Both rerankers return the mapped rows in input order. An unmapped row
/// retains the caller's current-storage score.
fn apply_reranked_scores(candidates: &mut [ScoredResult], rescored: &[ScoredResult]) {
    let mut next = rescored.iter().peekable();
    for row in candidates {
        if let Some(hit) = next.next_if(|hit| hit.id == row.id) {
            row.score = hit.score;
        }
    }
}
