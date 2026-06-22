//! Structured ORDER BY evaluation for MATCH results (EPIC-045 US-005).
//!
//! Split from `similarity.rs` to keep that file under the 500-NLOC bar and to
//! house the arithmetic / `similarity(field, $v)` evaluators, which reuse the
//! SELECT-side scoring helpers instead of re-implementing them.

use super::super::ordering::{evaluate_arithmetic, ScoreContext};
use super::MatchResult;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{ArithmeticExpr, OrderByExpr, SimilarityOrderBy};
use std::collections::HashMap;

impl Collection {
    /// Applies a structured ORDER BY expression to MATCH results.
    ///
    /// Supported: `similarity()` (search score), `depth`, a valid
    /// `alias.property` path, explicit `similarity(field, $v)`, and arithmetic
    /// over a property (e.g. `year - 2000`). Aggregates (no GROUP BY) and bare
    /// aliases are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`Error::GraphNotSupported`] (VELES-018) for unsupported
    /// expressions, or a parameter/storage error while resolving the query
    /// vector for `similarity(field, $v)`.
    pub(in crate::collection::search::query) fn order_match_results(
        &self,
        results: &mut [MatchResult],
        expr: &OrderByExpr,
        descending: bool,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        match expr {
            OrderByExpr::SimilarityBare => {
                Self::sort_match_by_score(results, descending);
                Ok(())
            }
            OrderByExpr::Field(f) if f == "depth" => {
                Self::sort_match_by_depth(results, descending);
                Ok(())
            }
            OrderByExpr::Field(f) => self.order_match_results_by_property(results, f, descending),
            OrderByExpr::Similarity(sim) => {
                self.sort_match_by_similarity(results, sim, descending, params)
            }
            OrderByExpr::Arithmetic(arith) => {
                self.sort_match_by_arithmetic(results, arith, descending);
                Ok(())
            }
            OrderByExpr::Aggregate(_) => Err(Error::GraphNotSupported(
                "MATCH ORDER BY aggregate expression is not supported (use \
                 similarity(), depth, alias.property, or arithmetic over properties)"
                    .to_string(),
            )),
        }
    }

    /// Sorts by the pre-computed match score (zero-arg `similarity()`).
    fn sort_match_by_score(results: &mut [MatchResult], descending: bool) {
        results.sort_unstable_by(|a, b| {
            let cmp = a.score.unwrap_or(0.0).total_cmp(&b.score.unwrap_or(0.0));
            Self::apply_direction(cmp, descending)
        });
    }

    /// Sorts by traversal depth.
    fn sort_match_by_depth(results: &mut [MatchResult], descending: bool) {
        results.sort_unstable_by(|a, b| Self::apply_direction(a.depth.cmp(&b.depth), descending));
    }

    /// Sorts by `similarity(field, $v)`: each bound node's vector vs the resolved
    /// query vector under the collection's configured metric.
    ///
    /// The per-node key is normalized so a LARGER key always means "more
    /// similar" — distance metrics (lower = closer, `!higher_is_better`) are
    /// negated — so `DESC` is most-similar-first regardless of metric, matching
    /// the SELECT-side `ORDER BY similarity()`. A node with a missing/mismatched
    /// vector sorts as least similar. The metric is read before the
    /// vector-storage lock (config precedes vector_storage in the lock order;
    /// see `CONCURRENCY_MODEL.md`).
    fn sort_match_by_similarity(
        &self,
        results: &mut [MatchResult],
        sim: &SimilarityOrderBy,
        descending: bool,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        let query_vector = Self::resolve_vector(&sim.vector, params)?;
        let metric = self.config.read().metric;
        let higher_is_better = metric.higher_is_better();
        let vector_storage = self.vector_storage.read();
        results.sort_unstable_by(|a, b| {
            let score = |r: &MatchResult| -> f32 {
                let Some(v) = vector_storage.retrieve(r.node_id).ok().flatten() else {
                    return f32::NEG_INFINITY;
                };
                if v.len() != query_vector.len() || v.is_empty() {
                    return f32::NEG_INFINITY;
                }
                let raw = metric.calculate(&v, &query_vector);
                if higher_is_better {
                    raw
                } else {
                    -raw
                }
            };
            Self::apply_direction(score(a).total_cmp(&score(b)), descending)
        });
        Ok(())
    }

    /// Sorts by an arithmetic expression over each result node's payload,
    /// reusing the SELECT-side [`evaluate_arithmetic`] (a property variable like
    /// `year` resolves against the bound node's payload).
    fn sort_match_by_arithmetic(
        &self,
        results: &mut [MatchResult],
        arith: &ArithmeticExpr,
        descending: bool,
    ) {
        let payload_storage = self.payload_storage.read();
        results.sort_unstable_by(|a, b| {
            let score = |r: &MatchResult| -> f32 {
                let payload = payload_storage.retrieve(r.node_id).ok().flatten();
                let ctx = ScoreContext::new(r.score.unwrap_or(0.0), payload.as_ref());
                evaluate_arithmetic(arith, &ctx)
            };
            Self::apply_direction(score(a).total_cmp(&score(b)), descending)
        });
    }
}
