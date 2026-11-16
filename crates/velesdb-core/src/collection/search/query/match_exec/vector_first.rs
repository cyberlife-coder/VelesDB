//! `VectorFirst` MATCH execution: the MATCH walk, seeded by a vector search.
//!
//! When the planner picks `VectorFirst` (see
//! [`MatchQueryPlanner::seeding_similarity`] for exactly when), the walk's
//! start nodes are not every node the first pattern node matches but the
//! nodes nearest the query vector of the seeding `similarity()` leaf, nearest
//! first. Everything else is the walk every MATCH runs: the whole pattern,
//! the whole WHERE (the seeding leaf included), each path its own row. So a
//! seeded MATCH returns what an unseeded one returns.
//!
//! The seeds grow until the walk has its `LIMIT` of rows, the collection is
//! exhausted, or the farthest seed already fails the seeding leaf — every
//! node beyond it is farther still, so none of them can start a row.

use super::where_eval::resolve_query_vector;
use super::{MatchResult, MatchSources, StartNodes};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::guardrails::QueryContext;
use crate::velesql::match_planner::MatchQueryPlanner;
use crate::velesql::MatchClause;
use std::collections::HashMap;

impl Collection {
    /// Executes a MATCH the planner chose `VectorFirst` for, starting with
    /// `top_k` seeds.
    ///
    /// # Errors
    ///
    /// Returns an error if the clause has no seeding `similarity()` leaf, its
    /// query vector cannot be resolved, the vector search fails, or the walk
    /// fails (guard rails included).
    pub(crate) fn execute_match_vector_first(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: &QueryContext,
        top_k: usize,
        sources: &MatchSources,
    ) -> Result<Vec<MatchResult>> {
        let (seed, limit) = MatchQueryPlanner::seeding_similarity(match_clause)
            .zip(match_clause.return_clause.limit)
            .ok_or_else(|| {
                Error::Query(
                    "VectorFirst requires a seeding similarity() condition and a LIMIT".to_string(),
                )
            })?;
        let query_vector = resolve_query_vector(&seed.vector, params)?;
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        // Metric snapshot (config, rank 1) before the walk takes any guard.
        let higher_is_better = self.storage.config.read().metric.higher_is_better();
        let total = self.len();

        let mut wanted = top_k.max(limit).max(1);
        loop {
            let seeds = self.search(&query_vector, wanted)?;
            let ids: Vec<u64> = seeds.iter().map(|seed| seed.point.id).collect();
            let rows = self.execute_match_from(
                match_clause,
                params,
                Some(ctx),
                sources,
                StartNodes::Seeded(&ids),
            )?;
            let exhausted = seeds.len() < wanted || wanted >= total;
            let past_threshold = seeds.last().is_some_and(|farthest| {
                !crate::velesql::similarity_holds(
                    seed.operator,
                    farthest.score,
                    seed.threshold,
                    higher_is_better,
                )
            });
            if rows.len() >= limit || exhausted || past_threshold {
                return Ok(rows);
            }
            wanted = wanted.saturating_mul(2).min(total);
        }
    }
}
