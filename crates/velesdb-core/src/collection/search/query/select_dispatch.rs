//! Internal dispatch helpers for SELECT query execution.
//!
//! Extracted from the main `query/mod.rs` to keep that file under 500 NLOC.
//! These methods handle CBO strategy, main SELECT dispatch, JOIN pushdown
//! analysis, and post-processing (DISTINCT / ORDER BY / LIMIT).
//!
//! MATCH-specific dispatch lives in `match_dispatch.rs` (Extract Module).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

use super::{distinct, pushdown, ExtractedComponents, MAX_LIMIT};

impl Collection {
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
