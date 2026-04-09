//! Pipeline helper methods for query execution.
//!
//! Extracted from `query/mod.rs` to keep file NLOC under 500.
//! All methods here are `impl Collection` helpers used by the
//! SELECT / MATCH execution pipeline.

use super::options::{ExtractedComponents, QueryFinalizationContext, MAX_LIMIT};
use super::vector_group_by;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

impl Collection {
    /// Dispatches a MATCH query if the query contains a `match_clause`.
    ///
    /// Returns `Ok(Some(results))` when the MATCH path was taken, `Ok(None)` otherwise.
    /// LET bindings are not yet supported with MATCH queries (v1.10) -- an explicit
    /// error is returned instead of silently discarding them.
    pub(super) fn try_dispatch_match(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Option<Vec<SearchResult>>> {
        let Some(match_clause) = query.match_clause.as_ref() else {
            return Ok(None);
        };
        if !query.let_bindings.is_empty() {
            return Err(crate::error::Error::Query(
                "LET bindings are not supported with MATCH queries in this version".to_string(),
            ));
        }
        Ok(Some(self.dispatch_match_query(
            match_clause,
            params,
            ctx,
        )?))
    }

    /// Computes the effective `(limit, fetch_limit)` from a SELECT statement.
    ///
    /// `limit` is the final row count requested by the user (capped at [`MAX_LIMIT`]).
    /// `fetch_limit` adds the OFFSET so that post-processing can skip rows and still
    /// return `limit` results.
    pub(super) fn compute_fetch_limit(stmt: &crate::velesql::SelectStatement) -> (usize, usize) {
        let limit = usize::try_from(stmt.limit.unwrap_or(10))
            .unwrap_or(MAX_LIMIT)
            .min(MAX_LIMIT);
        let offset_val = stmt
            .offset
            .map_or(0, |o| usize::try_from(o).unwrap_or(MAX_LIMIT));
        let fetch_limit = limit.saturating_add(offset_val).min(MAX_LIMIT);
        (limit, fetch_limit)
    }

    /// Attempts early-return paths or validates LET-binding compatibility.
    ///
    /// When `let_bindings` is empty, delegates to [`try_early_return_path`] for
    /// NOT-similarity, union, and sparse fast paths. When LET bindings are present,
    /// checks that the query shape is compatible -- unsupported shapes get an explicit
    /// error instead of silent fallthrough.
    ///
    /// Returns `Ok(Some(results))` if an early path was taken, `Ok(None)` to continue
    /// to the main dispatch.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_early_return_or_guard_let(
        &self,
        query: &crate::velesql::Query,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        fetch_limit: usize,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Option<Vec<SearchResult>>> {
        if query.let_bindings.is_empty() {
            return self.try_early_return_path(stmt, params, extracted, fetch_limit, ctx);
        }
        Self::validate_let_binding_support(extracted)?;
        Ok(None)
    }

    /// Validates that LET bindings are compatible with the extracted query shape.
    ///
    /// LET bindings require [`finalize_query_results`] which early-return paths
    /// bypass. Returns an explicit error for unsupported combinations.
    fn validate_let_binding_support(extracted: &ExtractedComponents) -> Result<()> {
        let unsupported = if extracted.sparse_vector_search.is_some() {
            Some("SPARSE_NEAR")
        } else if extracted.is_not_similarity_query {
            Some("NOT similarity()")
        } else if extracted.is_union_query {
            Some("OR/union")
        } else {
            None
        };
        if let Some(shape) = unsupported {
            return Err(crate::error::Error::Query(format!(
                "LET bindings are not supported with {shape} queries in this version"
            )));
        }
        Ok(())
    }

    /// Phase 1: Guard-rail pre-checks, context creation, and query validation.
    ///
    /// Creates a [`QueryContext`](crate::guardrails::QueryContext) with optional
    /// timeout override from `WITH (timeout_ms=N)`.
    pub(super) fn prepare_query_context(
        &self,
        query: &crate::velesql::Query,
        client_id: &str,
    ) -> Result<crate::guardrails::QueryContext> {
        self.guard_rails
            .pre_check(client_id)
            .map_err(crate::error::Error::from)?;

        let mut ctx = self.guard_rails.create_context();

        // WITH (timeout_ms=N) overrides the collection-level timeout for this query.
        if let Some(override_ms) = query
            .select
            .with_clause
            .as_ref()
            .and_then(crate::velesql::WithClause::get_timeout_ms)
        {
            ctx.limits.timeout_ms = override_ms;
        }

        crate::velesql::QueryValidator::validate(query)
            .map_err(|e| crate::error::Error::Query(e.to_string()))?;

        Ok(ctx)
    }

    /// Phase 3: Join analysis, guard-rail checks, post-processing, and stats update.
    pub(super) fn finalize_query_results(
        &self,
        results: &mut Vec<SearchResult>,
        fctx: &QueryFinalizationContext<'_>,
    ) -> Result<()> {
        self.analyze_join_pushdown(fctx.stmt);
        self.check_guardrails_and_record(fctx.ctx, results.len())?;

        *results = self.apply_select_postprocessing(
            fctx.stmt,
            std::mem::take(results),
            fctx.params,
            fctx.limit,
            fctx.let_bindings,
        )?;

        // Update QueryPlanner adaptive stats for vector/SELECT queries (Fix #8).
        if fctx.extracted.vector_search.is_some() {
            // Reason: u128->u64 cast; query durations < u64::MAX µs (~585 millennia)
            #[allow(clippy::cast_possible_truncation)]
            let vector_latency_us = fctx.ctx.elapsed().as_micros() as u64;
            self.query_planner
                .stats()
                .update_vector_latency(vector_latency_us);
        }
        self.guard_rails.circuit_breaker.record_success();
        Ok(())
    }

    /// Extracts all query components from the SELECT statement's WHERE clause.
    pub(super) fn extract_query_components(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<ExtractedComponents> {
        let mut vector_search = None;
        let mut similarity_conditions = Vec::new();
        let mut filter_condition = None;
        let mut graph_match_predicates = Vec::new();
        let mut sparse_vector_search = None;

        let is_union_query = stmt
            .where_clause
            .as_ref()
            .is_some_and(Self::has_similarity_in_problematic_or);
        let is_not_similarity_query = stmt
            .where_clause
            .as_ref()
            .is_some_and(Self::has_similarity_under_not);

        if let Some(ref cond) = stmt.where_clause {
            Self::validate_similarity_query_structure(cond)?;
            Self::collect_graph_match_predicates(cond, &mut graph_match_predicates);
            sparse_vector_search = Self::extract_sparse_vector_search(cond).cloned();

            let mut extracted_cond = cond.clone();
            vector_search = self.extract_vector_search(&mut extracted_cond, params)?;
            similarity_conditions =
                self.extract_all_similarity_conditions(&extracted_cond, params)?;
            filter_condition = Some(extracted_cond);
        }

        Ok(ExtractedComponents {
            vector_search,
            similarity_conditions,
            filter_condition,
            graph_match_predicates,
            sparse_vector_search,
            is_union_query,
            is_not_similarity_query,
        })
    }

    /// Applies vector-search GROUP BY post-processing on search results.
    ///
    /// Extracts aggregation functions from the SELECT columns and delegates
    /// to [`vector_group_by::group_search_results`].
    #[allow(clippy::unused_self)] // Instance method for consistency with other query pipeline stages.
    pub(super) fn apply_vector_group_by(
        &self,
        stmt: &crate::velesql::SelectStatement,
        results: &[SearchResult],
    ) -> Vec<SearchResult> {
        let group_by = stmt
            .group_by
            .as_ref()
            .expect("invariant: execute_vector_group_by_query requires stmt.group_by.is_some()");
        let aggregations = Self::extract_aggregations(&stmt.columns);
        let limit_hint = stmt.limit.map(|l| usize::try_from(l).unwrap_or(MAX_LIMIT));
        let config = vector_group_by::VectorGroupByConfig {
            group_by_columns: &group_by.columns,
            aggregations: &aggregations,
            limit_hint,
        };
        vector_group_by::group_search_results(results, &config)
    }

    /// Extracts aggregate functions from `SelectColumns`.
    fn extract_aggregations(
        columns: &crate::velesql::SelectColumns,
    ) -> Vec<crate::velesql::AggregateFunction> {
        match columns {
            crate::velesql::SelectColumns::Aggregations(aggs) => aggs.clone(),
            crate::velesql::SelectColumns::Mixed { aggregations, .. } => aggregations.clone(),
            _ => Vec::new(),
        }
    }

    /// Checks timeout and cardinality guard-rails, recording failure on violation.
    pub(super) fn check_guardrails_and_record(
        &self,
        ctx: &crate::guardrails::QueryContext,
        result_count: usize,
    ) -> Result<()> {
        ctx.check_timeout()
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        ctx.check_cardinality(result_count)
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        Ok(())
    }

    /// Executes a query with instrumentation and returns plan + actual stats.
    ///
    /// Builds the plan, times execution, and collects per-node statistics.
    /// Used by `VectorCollection` and `GraphCollection` newtypes.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is invalid or execution fails.
    pub fn explain_analyze_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::ExplainOutput> {
        use crate::velesql::{build_leaf_node_stats, ActualStats, ExplainOutput, QueryPlan};

        // Use from_query() (not from_select/from_match) to include LET bindings,
        // keeping the plan consistent with the Database-level explain path.
        // Cache fields are unavailable at the Collection level and remain None.
        let plan = QueryPlan::from_query(query);

        let start = std::time::Instant::now();
        let results = self.execute_query(query, params)?;
        let elapsed = start.elapsed();

        let actual_rows = results.len() as u64;
        let actual_time_ms = elapsed.as_secs_f64() * 1000.0;
        let (nodes_visited, edges_traversed) = if query.is_match_query() {
            (actual_rows, actual_rows)
        } else {
            (0, 0)
        };

        let stats = ActualStats {
            actual_rows,
            actual_time_ms,
            loops: 1,
            nodes_visited,
            edges_traversed,
        };

        let node_stats = build_leaf_node_stats(&plan.root, actual_rows, actual_time_ms);
        Ok(ExplainOutput::with_stats(plan, stats, node_stats))
    }
}
