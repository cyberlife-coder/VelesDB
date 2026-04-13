//! Internal dispatch helpers for SELECT query execution.
//!
//! Extracted from the main `query/mod.rs` to keep that file under 500 NLOC.
//! These methods handle MATCH dispatch, CBO strategy, main SELECT dispatch,
//! JOIN pushdown analysis, and post-processing (DISTINCT / ORDER BY / LIMIT).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

use super::{distinct, pushdown, ExtractedComponents, MAX_LIMIT};

/// Global MATCH query metrics collector (EPIC-050).
///
/// Uses `LazyLock` for thread-safe one-time initialisation.
/// Per-collection metrics registries are a future enhancement.
static MATCH_METRICS: std::sync::LazyLock<super::match_metrics::MatchMetrics> =
    std::sync::LazyLock::new(super::match_metrics::MatchMetrics::new);

impl Collection {
    /// Computes collection statistics for MATCH query planning.
    ///
    /// Gathers node count, edge count, average degree, and label statistics
    /// from the live collection data structures for cost-based strategy selection.
    // Reason: usize->f64 casts are for cost-estimation ratios, not precise calculations.
    #[allow(clippy::cast_precision_loss)]
    fn compute_match_collection_stats(&self) -> super::match_planner::CollectionStats {
        let total_nodes = self.len();
        let total_edges = self.edge_store.len();
        let avg_degree = if total_nodes > 0 {
            total_edges as f64 / total_nodes as f64
        } else {
            0.0
        };
        let label_count = self.edge_store.label_count();
        let label_selectivity = if label_count > 0 {
            1.0 / label_count as f64
        } else {
            1.0
        };
        super::match_planner::CollectionStats {
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
    /// records query metrics via the global `MATCH_METRICS` collector, then
    /// delegates to the graph traversal engine.
    pub(super) fn dispatch_match_query(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let start = std::time::Instant::now();

        // W6-A2: Cost-based strategy selection.
        let stats = self.compute_match_collection_stats();
        let strategy = super::match_planner::MatchQueryPlanner::plan(match_clause, &stats);
        tracing::debug!(strategy = ?strategy, "MATCH execution strategy selected");

        // Dispatch based on strategy.
        let result = match &strategy {
            super::match_planner::MatchExecutionStrategy::VectorFirst {
                similarity_alias,
                top_k,
                threshold,
            } => {
                let vf_results = self.execute_match_vector_first(
                    match_clause,
                    params,
                    ctx,
                    similarity_alias,
                    *top_k,
                    *threshold,
                )?;
                self.finalize_match_results(match_clause, vf_results, ctx)
            }
            super::match_planner::MatchExecutionStrategy::Parallel {
                ref vector_hint, ..
            } => self.execute_match_parallel(match_clause, params, ctx, vector_hint),
            super::match_planner::MatchExecutionStrategy::GraphFirst { .. } => {
                self.execute_match_pipeline(match_clause, params, ctx)
            }
        };

        // W6-A3: Record metrics.
        let max_depth = super::match_planner::MatchQueryPlanner::count_hops(match_clause);
        match &result {
            Ok(results) => {
                MATCH_METRICS.record_success(start.elapsed(), results.len(), max_depth);
            }
            Err(_) => {
                MATCH_METRICS.record_failure(start.elapsed());
            }
        }

        result
    }

    /// Executes the MATCH pipeline: traversal, ordering, conversion, and limits.
    ///
    /// Factored out of `dispatch_match_query` so metrics recording wraps the
    /// entire operation cleanly.
    fn execute_match_pipeline(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let match_results = self.execute_match_with_context(match_clause, params, Some(ctx))?;
        self.finalize_match_results(match_clause, match_results, ctx)
    }

    /// Applies ORDER BY, conversion to `SearchResult`, cardinality check,
    /// LIMIT, and latency recording to a set of `MatchResult`s.
    ///
    /// Executes the Parallel MATCH strategy (Wave 6 Phase D).
    ///
    /// Runs GraphFirst and VectorFirst sequentially, then merges the result
    /// sets by `node_id` (union semantics -- best score wins for duplicates).
    ///
    /// True parallel execution (rayon/tokio) is a future optimisation; the
    /// sequential approach is correct and avoids concurrency complexity for
    /// typical MATCH query sizes.
    fn execute_match_parallel(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
        vector_hint: &super::match_planner::MatchExecutionStrategy,
    ) -> Result<Vec<SearchResult>> {
        // Phase 1: GraphFirst path.
        let graph_results = self.execute_match_with_context(match_clause, params, Some(ctx))?;

        // Phase 2: VectorFirst path (extract hint parameters).
        let vector_results = if let super::match_planner::MatchExecutionStrategy::VectorFirst {
            similarity_alias,
            top_k,
            threshold,
        } = vector_hint
        {
            self.execute_match_vector_first(
                match_clause,
                params,
                ctx,
                similarity_alias,
                *top_k,
                *threshold,
            )?
        } else {
            tracing::warn!(
                "Parallel strategy vector_hint is not VectorFirst; \
                     skipping vector path"
            );
            Vec::new()
        };

        // Phase 3: Merge by node_id (union, best score wins).
        let merged = merge_match_results(graph_results, vector_results);
        self.finalize_match_results(match_clause, merged, ctx)
    }

    /// Shared by GraphFirst, VectorFirst, and Parallel strategies.
    fn finalize_match_results(
        &self,
        match_clause: &crate::velesql::MatchClause,
        match_results: Vec<super::match_exec::MatchResult>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
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
        let search_opts = super::QuerySearchOptions::from_with_clause(stmt.with_clause.as_ref())
            .with_fusion(stmt.fusion_clause.clone());
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
                &search_opts,
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
    ///
    /// Returns a [`PushdownAnalysis`](pushdown::PushdownAnalysis) classifying
    /// WHERE conditions by data source so the caller can route each filter to
    /// the correct execution stage.
    #[allow(clippy::unused_self)]
    pub(super) fn analyze_join_pushdown(
        &self,
        stmt: &crate::velesql::SelectStatement,
    ) -> pushdown::PushdownAnalysis {
        if stmt.joins.is_empty() {
            return pushdown::PushdownAnalysis::default();
        }
        let Some(ref cond) = stmt.where_clause else {
            return pushdown::PushdownAnalysis::default();
        };
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
        analysis
    }

    /// Applies DISTINCT, ORDER BY (with LET bindings), OFFSET, LIMIT, and
    /// LET payload injection (Issue #473).
    pub(super) fn apply_select_postprocessing(
        &self,
        stmt: &crate::velesql::SelectStatement,
        mut results: Vec<SearchResult>,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
        let_bindings: &[crate::velesql::LetBinding],
    ) -> Result<Vec<SearchResult>> {
        if stmt.distinct == crate::velesql::DistinctMode::All {
            results = distinct::apply_distinct(results, &stmt.columns);
        }
        if let Some(ref order_by) = stmt.order_by {
            if let_bindings.is_empty() {
                self.apply_order_by(&mut results, order_by, params)?;
            } else {
                let per_result_let = Self::evaluate_let_for_results(let_bindings, &results);
                self.apply_order_by_with_let(&mut results, order_by, params, &per_result_let)?;
            }
        }
        // SQL-standard: OFFSET applied after ORDER BY, before LIMIT.
        if let Some(offset) = stmt.offset {
            let skip = usize::try_from(offset).unwrap_or(usize::MAX);
            results = results.into_iter().skip(skip).collect();
        }
        results.truncate(limit);

        // Issue #473: Inject LET binding values into result payloads so they
        // appear in SELECT projection and API responses.
        if !let_bindings.is_empty() {
            let per_result_let = Self::evaluate_let_for_results(let_bindings, &results);
            inject_let_into_payloads(&mut results, &per_result_let);
        }

        Ok(results)
    }

    /// Evaluates LET bindings for every result, producing per-result binding maps.
    fn evaluate_let_for_results(
        let_bindings: &[crate::velesql::LetBinding],
        results: &[SearchResult],
    ) -> Vec<Vec<(String, f32)>> {
        results
            .iter()
            .map(|r| {
                super::ordering::evaluate_let_bindings(
                    let_bindings,
                    r.score,
                    r.point.payload.as_ref(),
                    r.component_scores.as_deref(),
                )
            })
            .collect()
    }
}

/// Injects evaluated LET binding values into each result's payload.
///
/// This makes LET bindings visible in SELECT projection and API responses.
/// LET bindings take precedence over payload fields with the same name.
fn inject_let_into_payloads(results: &mut [SearchResult], per_result_let: &[Vec<(String, f32)>]) {
    for (result, bindings) in results.iter_mut().zip(per_result_let.iter()) {
        if bindings.is_empty() {
            continue;
        }
        let payload = result
            .point
            .payload
            .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = payload {
            for (name, value) in bindings {
                map.insert(name.clone(), serde_json::Value::from(f64::from(*value)));
            }
        }
    }
}

/// Merges two sets of `MatchResult`s by `node_id` (union semantics).
///
/// When both sets contain the same `node_id`, the result with the higher
/// `score` is kept. Results without a score compare as `f32::NEG_INFINITY`.
/// The merged output is sorted by score descending.
fn merge_match_results(
    graph_results: Vec<super::match_exec::MatchResult>,
    vector_results: Vec<super::match_exec::MatchResult>,
) -> Vec<super::match_exec::MatchResult> {
    use std::collections::HashMap;

    let mut by_node: HashMap<u64, super::match_exec::MatchResult> =
        HashMap::with_capacity(graph_results.len() + vector_results.len());

    for r in graph_results {
        by_node.insert(r.node_id, r);
    }

    for r in vector_results {
        let node_id = r.node_id;
        match by_node.entry(node_id) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let new_score = r.score.unwrap_or(f32::NEG_INFINITY);
                let old_score = entry.get().score.unwrap_or(f32::NEG_INFINITY);
                if new_score > old_score {
                    entry.insert(r);
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(r);
            }
        }
    }

    let mut merged: Vec<super::match_exec::MatchResult> = by_node.into_values().collect();
    merged.sort_by(|a, b| {
        let sa = a.score.unwrap_or(f32::NEG_INFINITY);
        let sb = b.score.unwrap_or(f32::NEG_INFINITY);
        sb.total_cmp(&sa)
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::super::match_exec::MatchResult;
    use super::merge_match_results;

    fn mr(node_id: u64, score: Option<f32>) -> MatchResult {
        let mut r = MatchResult::new(node_id, 0, Vec::new());
        r.score = score;
        r
    }

    #[test]
    fn test_merge_empty_inputs() {
        let merged = merge_match_results(Vec::new(), Vec::new());
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_graph_only() {
        let graph = vec![mr(1, None), mr(2, Some(0.5))];
        let merged = merge_match_results(graph, Vec::new());
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].node_id, 2);
    }

    #[test]
    fn test_merge_vector_only() {
        let vector = vec![mr(3, Some(0.9)), mr(4, Some(0.7))];
        let merged = merge_match_results(Vec::new(), vector);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].node_id, 3);
        assert_eq!(merged[1].node_id, 4);
    }

    #[test]
    fn test_merge_union_distinct_nodes() {
        let graph = vec![mr(1, None), mr(2, None)];
        let vector = vec![mr(3, Some(0.8)), mr(4, Some(0.6))];
        let merged = merge_match_results(graph, vector);
        assert_eq!(merged.len(), 4);
    }

    #[test]
    fn test_merge_duplicate_keeps_higher_score() {
        let graph = vec![mr(1, Some(0.3))];
        let vector = vec![mr(1, Some(0.9))];
        let merged = merge_match_results(graph, vector);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].node_id, 1);
        assert!((merged[0].score.expect("test: should have score") - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_merge_duplicate_graph_wins_when_higher() {
        let graph = vec![mr(1, Some(0.95))];
        let vector = vec![mr(1, Some(0.5))];
        let merged = merge_match_results(graph, vector);
        assert_eq!(merged.len(), 1);
        assert!((merged[0].score.expect("test: should have score") - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn test_merge_sorted_descending() {
        let graph = vec![mr(1, Some(0.3)), mr(2, Some(0.1))];
        let vector = vec![mr(3, Some(0.9)), mr(4, Some(0.5))];
        let merged = merge_match_results(graph, vector);
        let scores: Vec<f32> = merged
            .iter()
            .map(|r| r.score.unwrap_or(f32::NEG_INFINITY))
            .collect();
        for w in scores.windows(2) {
            assert!(w[0] >= w[1], "scores should be descending: {scores:?}");
        }
    }

    #[test]
    fn test_merge_none_scores_sorted_last() {
        let graph = vec![mr(1, None), mr(2, None)];
        let vector = vec![mr(3, Some(0.5))];
        let merged = merge_match_results(graph, vector);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].node_id, 3);
    }
}
