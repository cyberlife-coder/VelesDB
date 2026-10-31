//! Internal dispatch helpers for MATCH query execution.
//!
//! Extracted from `select_dispatch.rs` (Martin Fowler: Extract Module) to keep
//! file NLOC under 500. These methods handle MATCH dispatch, parallel
//! execution, result merging, and MATCH-specific metrics.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

use super::match_metrics::global_match_metrics;
use super::MAX_LIMIT;

impl Collection {
    /// Computes collection statistics for MATCH query planning.
    ///
    /// Gathers node count, edge count, average degree, and label statistics
    /// from the live collection data structures for cost-based strategy selection.
    // Reason: usize->f64 casts are for cost-estimation ratios, not precise calculations.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn compute_match_collection_stats(
        &self,
    ) -> crate::velesql::match_planner::MatchGraphStats {
        let total_nodes = self.len();
        let total_edges = self.graph.edge_store.len();
        let avg_degree = if total_nodes > 0 {
            total_edges as f64 / total_nodes as f64
        } else {
            0.0
        };
        let label_count = self.graph.edge_store.label_count();
        let label_selectivity = if label_count > 0 {
            1.0 / label_count as f64
        } else {
            1.0
        };
        crate::velesql::match_planner::MatchGraphStats {
            total_nodes,
            total_edges,
            avg_degree,
            label_count,
            label_selectivity,
        }
    }

    /// Dispatches a MATCH query through the graph traversal path.
    ///
    /// Calls the cost-based `MatchQueryPlanner` to select an execution strategy,
    /// records query metrics via [`global_match_metrics`], then delegates to
    /// the graph traversal engine.
    pub(super) fn dispatch_match_query(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let raw = self.dispatch_match_strategy(match_clause, params, ctx);
        count_guard_rail_stop(
            raw.and_then(|raw| self.finalize_match_results(match_clause, raw, ctx, params)),
        )
    }

    /// Public ordered-MATCH entry point: runs the full cost-based planner
    /// pipeline (guard-rail pre-check, strategy selection, metrics, RETURN
    /// `ORDER BY` with deterministic tie-break, and post-sort LIMIT) and
    /// returns ordered [`MatchResult`](super::match_exec::MatchResult)s.
    ///
    /// This is the SINGLE method non-SQL surfaces (REST `/match`, the Python /
    /// TypeScript SDKs) should call so they rank identically to the SQL `/query`
    /// path instead of re-implementing ordering or returning raw traversal order
    /// (backlog #1). Unlike the backward-compatible [`execute_match`] /
    /// [`execute_match_with_similarity`] entry points (which run without a
    /// guard-rail context), this routes through the planner and enforces
    /// guard-rails.
    ///
    /// [`execute_match`]: Self::execute_match
    /// [`execute_match_with_similarity`]: Self::execute_match_with_similarity
    ///
    /// # Errors
    ///
    /// Returns an error if a guard-rail pre-check fails, or if traversal,
    /// ordering, or a guard-rail check during execution fails.
    pub fn match_query_ordered(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<super::match_exec::MatchResult>> {
        self.runtime
            .guard_rails
            .pre_check("default")
            .map_err(crate::error::Error::from)?;
        validate_match_clause(match_clause)?;
        let ctx = self.runtime.guard_rails.create_context();
        self.dispatch_match_ordered(match_clause, params, &ctx)
    }

    /// Runs the cost-based MATCH planner and the selected execution strategy,
    /// returning the ordered, post-sort-LIMITed [`MatchResult`]s **before**
    /// conversion to [`SearchResult`].
    ///
    /// Single source of truth for non-SQL surfaces (REST `/match`, the SDKs)
    /// that need ordered graph rows: it shares the SAME planner, metrics,
    /// deterministic tie-break, and post-sort LIMIT as the SQL `/query` path
    /// ([`dispatch_match_query`](Self::dispatch_match_query)), the only
    /// difference being the return type (`MatchResult` vs the converted
    /// `SearchResult`). Without it those surfaces re-implement ordering or
    /// return raw traversal order (backlog #1).
    ///
    /// # Errors
    ///
    /// Returns an error if traversal, ordering, or a guard-rail check fails.
    ///
    /// [`MatchResult`]: super::match_exec::MatchResult
    pub(in crate::collection::search::query) fn dispatch_match_ordered(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<super::match_exec::MatchResult>> {
        let raw = self.dispatch_match_strategy(match_clause, params, ctx);
        count_guard_rail_stop(
            raw.and_then(|raw| self.finalize_match_ordering(match_clause, raw, ctx, params)),
        )
    }

    /// Selects and runs the planner strategy, returning RAW (unordered,
    /// unconverted) [`MatchResult`]s plus recording metrics. Shared by the SQL
    /// `SearchResult` path and the ordered `MatchResult` path so strategy
    /// dispatch lives in exactly one place.
    ///
    /// [`MatchResult`]: super::match_exec::MatchResult
    fn dispatch_match_strategy(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<super::match_exec::MatchResult>> {
        let start = std::time::Instant::now();
        if crate::velesql::match_planner::MatchQueryPlanner::has_similarity_condition(
            match_clause.where_clause.as_ref(),
        ) {
            global_match_metrics().record_similarity_query();
        }

        // W6-A2: Cost-based strategy selection.
        let stats = self.compute_match_collection_stats();
        let strategy = crate::velesql::match_planner::MatchQueryPlanner::plan(match_clause, &stats);
        tracing::debug!(strategy = ?strategy, "MATCH execution strategy selected");

        // A traversal failure never reaches the finalizing steps that record
        // the breaker outcome, so it is recorded here.
        let result = self
            .run_match_strategy(match_clause, params, ctx, &strategy)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e));

        // W6-A3: Record metrics.
        let max_depth = crate::velesql::match_planner::MatchQueryPlanner::count_hops(match_clause);
        match &result {
            Ok(results) => {
                global_match_metrics().record_success(start.elapsed(), results.len(), max_depth);
            }
            Err(_) => {
                global_match_metrics().record_failure(start.elapsed());
            }
        }

        result
    }

    /// Dispatches to the strategy-specific traversal, returning RAW results.
    fn run_match_strategy(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
        strategy: &crate::velesql::match_planner::MatchExecutionStrategy,
    ) -> Result<Vec<super::match_exec::MatchResult>> {
        match strategy {
            crate::velesql::match_planner::MatchExecutionStrategy::VectorFirst {
                similarity_alias,
                top_k,
                threshold,
            } => self.execute_match_vector_first(
                match_clause,
                params,
                ctx,
                similarity_alias,
                *top_k,
                *threshold,
            ),
            crate::velesql::match_planner::MatchExecutionStrategy::GraphFirst { .. } => {
                self.execute_match_with_context(match_clause, params, Some(ctx))
            }
        }
    }

    /// Applies ORDER BY, conversion to `SearchResult`, cardinality check,
    /// LIMIT, and latency recording to a set of `MatchResult`s.
    ///
    /// Shared by the GraphFirst and VectorFirst strategies.
    fn finalize_match_results(
        &self,
        match_clause: &crate::velesql::MatchClause,
        match_results: Vec<super::match_exec::MatchResult>,
        ctx: &crate::guardrails::QueryContext,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        ctx.check_timeout()
            .map_err(crate::error::Error::from)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;

        let mut sorted = match_results;
        self.apply_match_order_by(&mut sorted, match_clause, params)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;

        let mut results = self
            .match_results_to_search_results(sorted)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        // Final cardinality check for MATCH path (EPIC-048 US-003).
        ctx.check_cardinality(results.len())
            .map_err(crate::error::Error::from)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        if let Some(limit) = match_return_limit(match_clause) {
            results.truncate(limit);
        }
        // Reason: u128->u64 cast; query durations < u64::MAX µs (~585 millennia)
        #[allow(clippy::cast_possible_truncation)]
        let graph_latency_us = ctx.elapsed().as_micros() as u64;
        self.query
            .query_planner
            .stats()
            .update_graph_latency(graph_latency_us);
        self.runtime.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }

    /// Applies the timeout guard, RETURN `ORDER BY` (deterministic tie-break),
    /// and the post-sort LIMIT to raw `MatchResult`s, returning ordered rows
    /// WITHOUT converting to `SearchResult`.
    ///
    /// Shares the exact ordering ([`apply_match_order_by`](Self::apply_match_order_by))
    /// and LIMIT ([`match_return_limit`]) logic with the SQL `SearchResult`
    /// finalize path, so the ordered `MatchResult` surface ranks identically.
    fn finalize_match_ordering(
        &self,
        match_clause: &crate::velesql::MatchClause,
        match_results: Vec<super::match_exec::MatchResult>,
        ctx: &crate::guardrails::QueryContext,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<super::match_exec::MatchResult>> {
        ctx.check_timeout()
            .map_err(crate::error::Error::from)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;

        let mut sorted = match_results;
        self.apply_match_order_by(&mut sorted, match_clause, params)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;

        // Final cardinality check for MATCH path (EPIC-048 US-003), matching
        // `finalize_match_results` so the ordered surface rejects oversized
        // result sets identically to the SQL path.
        ctx.check_cardinality(sorted.len())
            .map_err(crate::error::Error::from)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        if let Some(limit) = match_return_limit(match_clause) {
            sorted.truncate(limit);
        }
        self.runtime.guard_rails.circuit_breaker.record_success();
        Ok(sorted)
    }

    /// Applies RETURN `ORDER BY` (with the deterministic `(node_id, depth, path)`
    /// tie-break baseline) to raw MATCH results in place. Sorts only when an
    /// ORDER BY is present, so traversal-order output is otherwise preserved.
    ///
    /// Single source of truth shared by the SQL `/query` finalize path and the
    /// direct `execute_match` / `execute_match_with_similarity` entry points
    /// (REST `/match`, the SDKs) so every surface orders identically.
    pub(in crate::collection::search::query) fn apply_match_order_by(
        &self,
        results: &mut [super::match_exec::MatchResult],
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        if let Some(order_by) = match_clause.return_clause.order_by.as_ref() {
            sort_match_baseline(results);
            for item in order_by.iter().rev() {
                self.order_match_results(results, &item.expr, item.descending, params)?;
            }
        }
        Ok(())
    }
}

/// Refuses a MATCH clause `QueryValidator` refuses, before any traversal,
/// with the code `Database::execute_query` gives the same clause inside a
/// statement (`VELES-010` carrying the validation code). The entry points
/// that take a bare `MatchClause` call it, so a MATCH is validated by one rule
/// set whichever surface it arrives through.
pub(in crate::collection::search::query) fn validate_match_clause(
    match_clause: &crate::velesql::MatchClause,
) -> Result<()> {
    crate::velesql::QueryValidator::validate_match_clause(match_clause)
        .map_err(|e| crate::error::Error::Query(e.to_string()))
}

/// Counts a MATCH stopped by a guard rail during execution — depth,
/// cardinality, memory or timeout, raised by the strategy or by the final
/// checks — in `velesdb_match_guardrail_hits_total`, and passes the result on.
///
/// Both MATCH funnels ([`Collection::dispatch_match_query`] and
/// [`Collection::dispatch_match_ordered`]) end here, so each stop is counted
/// once. A rate-limit or circuit-breaker refusal happens before a MATCH
/// starts and is not one of these (see `velesdb_rate_limit_requests_total`).
fn count_guard_rail_stop<T>(result: Result<T>) -> Result<T> {
    if matches!(result, Err(crate::error::Error::GuardRail { .. })) {
        global_match_metrics().record_guard_rail_hit();
    }
    result
}

/// Computes the effective RETURN `LIMIT` for a MATCH query, clamped to the
/// server-wide `MAX_LIMIT` ceiling. `None` means no LIMIT was specified, so the
/// caller leaves the result set unbounded (subject only to `MAX_LIMIT` upstream).
pub(in crate::collection::search::query) fn match_return_limit(
    match_clause: &crate::velesql::MatchClause,
) -> Option<usize> {
    match_clause
        .return_clause
        .limit
        .map(|l| usize::try_from(l).unwrap_or(MAX_LIMIT).min(MAX_LIMIT))
}

/// Deterministic ORDER BY tie-break baseline keyed by `(node_id, depth, path)` —
/// a total order over connected matches: a single-node match has a unique
/// `node_id` (empty path); a multi-node match is fixed by its edge-id `path`
/// (edge ids are unique, so the path determines the whole route). `node_id`
/// alone is NOT unique for multi-node patterns (the matched node repeats across
/// results that differ only in their bindings). Applied before the stable
/// per-column sorts so rows equal on every ORDER BY key order deterministically.
fn sort_match_baseline(results: &mut [super::match_exec::MatchResult]) {
    results.sort_unstable_by(|a, b| {
        a.node_id
            .cmp(&b.node_id)
            .then_with(|| a.depth.cmp(&b.depth))
            .then_with(|| a.path.cmp(&b.path))
    });
}
