//! Internal dispatch helpers for SELECT query execution.
//!
//! Extracted from the main `query/mod.rs` to keep that file under 500 NLOC.
//! These methods handle MATCH dispatch, CBO strategy, main SELECT dispatch,
//! JOIN pushdown analysis, and post-processing (DISTINCT / ORDER BY / LIMIT).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

use super::{distinct, pushdown, ExtractedComponents, MAX_LIMIT};

impl Collection {
    /// Dispatches a MATCH query through the graph traversal path.
    pub(super) fn dispatch_match_query(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let match_results = self.execute_match_with_context(match_clause, params, Some(ctx))?;

        ctx.check_timeout()
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

        let mut sorted = match_results;
        if let Some(order_by) = match_clause.return_clause.order_by.as_ref() {
            for item in order_by.iter().rev() {
                self.order_match_results(&mut sorted, &item.expression, item.descending);
            }
        }

        let mut results = self
            .match_results_to_search_results(sorted)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        // Final cardinality check for MATCH path (EPIC-048 US-003).
        ctx.check_cardinality(results.len())
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        if let Some(limit) = match_clause.return_clause.limit {
            let limit = usize::try_from(limit).unwrap_or(MAX_LIMIT).min(MAX_LIMIT);
            results.truncate(limit);
        }
        // Reason: u128->u64 cast; query durations < u64::MAX µs (~585 millennia)
        #[allow(clippy::cast_possible_truncation)]
        let graph_latency_us = ctx.elapsed().as_micros() as u64;
        self.query_planner
            .stats()
            .update_graph_latency(graph_latency_us);
        self.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }

    /// Computes the CBO execution strategy and over-fetch factor for the query.
    pub(super) fn compute_cbo_strategy(
        &self,
        filter_condition: Option<&crate::velesql::Condition>,
        limit: usize,
    ) -> (crate::velesql::ExecutionStrategy, usize) {
        let col_stats = self.get_stats();
        let result = self.query_planner.choose_strategy_with_cbo_and_overfetch(
            &col_stats,
            filter_condition,
            limit,
        );
        tracing::debug!(
            strategy = ?result.0, over_fetch = result.1,
            "CBO selected execution strategy"
        );
        result
    }

    /// Dispatches the main SELECT query path (vector, similarity, metadata).
    pub(super) fn dispatch_main_select(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        _ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let has_graph_predicates = !extracted.graph_match_predicates.is_empty();
        let skip_metadata_prefilter_for_graph_or = has_graph_predicates
            && stmt
                .where_clause
                .as_ref()
                .is_some_and(Self::condition_contains_or);
        let execution_limit = if has_graph_predicates {
            MAX_LIMIT
        } else {
            limit
        };
        let ef_search = stmt
            .with_clause
            .as_ref()
            .and_then(crate::velesql::WithClause::get_ef_search);
        let first_similarity = extracted.similarity_conditions.first().cloned();
        let (cbo_strategy, cbo_over_fetch) =
            self.compute_cbo_strategy(extracted.filter_condition.as_ref(), limit);

        let mut results = self
            .dispatch_vector_query(
                extracted.vector_search.as_ref(),
                first_similarity.as_ref(),
                &extracted.similarity_conditions,
                extracted.filter_condition.as_ref(),
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
                ef_search,
                cbo_strategy,
                cbo_over_fetch,
            )
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

        if has_graph_predicates {
            if let Some(cond) = stmt.where_clause.as_ref() {
                results = self
                    .apply_where_condition_to_results(results, cond, params, &stmt.from_alias)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
            }
        }

        Ok(results)
    }

    /// Analyzes JOIN pushdown opportunities (EPIC-031 US-006).
    #[allow(clippy::unused_self)]
    pub(super) fn analyze_join_pushdown(&self, stmt: &crate::velesql::SelectStatement) {
        if stmt.joins.is_empty() {
            return;
        }
        if let Some(ref cond) = stmt.where_clause {
            let graph_vars: std::collections::HashSet<String> =
                stmt.from_alias.iter().cloned().collect();
            let join_tables = pushdown::extract_join_tables(&stmt.joins);
            let analysis = pushdown::analyze_for_pushdown(cond, &graph_vars, &join_tables);
            tracing::debug!(
                column_store_filters = analysis.column_store_filters.len(),
                graph_filters = analysis.graph_filters.len(),
                post_join_filters = analysis.post_join_filters.len(),
                has_pushdown = analysis.has_pushdown(),
                "JOIN pushdown analysis complete"
            );
        }
    }

    /// Applies DISTINCT, ORDER BY, and LIMIT to SELECT results.
    pub(super) fn apply_select_postprocessing(
        &self,
        stmt: &crate::velesql::SelectStatement,
        mut results: Vec<SearchResult>,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if stmt.distinct == crate::velesql::DistinctMode::All {
            results = distinct::apply_distinct(results, &stmt.columns);
        }
        if let Some(ref order_by) = stmt.order_by {
            self.apply_order_by(&mut results, order_by, params)?;
        }
        results.truncate(limit);
        Ok(results)
    }
}
