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

    /// Like [`execute_query`](Self::execute_query) but also returns the
    /// graph-traversal counters `(nodes_visited, edges_traversed)` measured
    /// during MATCH execution. Non-MATCH (SELECT/vector) queries report
    /// `(_, 0, 0)`. Used by EXPLAIN ANALYZE to report real counts instead of a
    /// result-row proxy.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed.
    pub(crate) fn execute_query_counted(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(Vec<SearchResult>, u64, u64)> {
        // Only a standalone MATCH query carries traversal counters. Run it
        // through the same context + dispatch as execute_query, then read the
        // counters the executor recorded into the query context. is_match_query
        // guarantees try_dispatch_match returns Some.
        if query.is_match_query() && query.compound.is_none() {
            let ctx = self.prepare_query_context(query, "default")?;
            let results = self
                .try_dispatch_match(query, params, &ctx)?
                .unwrap_or_default();
            return Ok((
                results,
                ctx.traversal_nodes_visited(),
                ctx.traversal_edges_traversed(),
            ));
        }
        Ok((self.execute_query(query, params)?, 0, 0))
    }

    /// Computes the effective `(limit, fetch_limit)` from a SELECT statement.
    ///
    /// `limit` is the final row count requested by the user (capped at [`MAX_LIMIT`]);
    /// without an explicit LIMIT clause the engine default
    /// [`DEFAULT_SELECT_LIMIT`](crate::velesql::DEFAULT_SELECT_LIMIT) applies.
    /// `fetch_limit` adds the OFFSET so that post-processing can skip rows and still
    /// return `limit` results.
    pub(super) fn compute_fetch_limit(stmt: &crate::velesql::SelectStatement) -> (usize, usize) {
        let limit = usize::try_from(stmt.limit.unwrap_or(crate::velesql::DEFAULT_SELECT_LIMIT))
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
        } else if extracted.fused_search.is_some() {
            // NEAR_FUSED routes through the fused early-return path, which LET
            // bypasses — without this guard the fused vectors would be silently
            // dropped to a non-fused scan (same class as the SPARSE_NEAR guard).
            Some("NEAR_FUSED")
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
        self.runtime
            .guard_rails
            .pre_check(client_id)
            .map_err(crate::error::Error::from)?;

        let mut ctx = self.runtime.guard_rails.create_context();

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
        // Run pushdown analysis for diagnostic tracing at Collection level.
        // The actual pushdown execution happens in Database::execute_single_select().
        let _analysis = self.analyze_join_pushdown(fctx.stmt);
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
            let elapsed = fctx.ctx.elapsed();
            // Reason: u128->u64 cast; query durations < u64::MAX µs (~585 millennia)
            #[allow(clippy::cast_possible_truncation)]
            let vector_latency_us = elapsed.as_micros() as u64;
            self.query
                .query_planner
                .stats()
                .update_vector_latency(vector_latency_us);

            // Issue #469: CBO calibration feedback — record actual ms vs cost estimate.
            let actual_ms = elapsed.as_secs_f64() * 1000.0;
            let dataset_size = self.len();
            let ef_search = fctx
                .stmt
                .with_clause
                .as_ref()
                .and_then(crate::velesql::WithClause::get_ef_search)
                .unwrap_or(100);
            self.query
                .query_planner
                .record_cbo_feedback(dataset_size, ef_search, actual_ms);
        }
        self.runtime.guard_rails.circuit_breaker.record_success();
        Ok(())
    }

    /// Returns a copy of `stmt` with scalar WHERE parameter placeholders
    /// resolved, or `None` when the statement has no WHERE clause.
    ///
    /// Resolving once at pipeline entry guarantees every downstream
    /// conversion to a payload [`Filter`](crate::filter::Filter) sees bound
    /// values: without this, `Value::Parameter` silently degrades to JSON
    /// `null` in `filter::Condition::from`, returning 0 rows without error.
    ///
    /// # Errors
    ///
    /// Returns an error when a referenced parameter is missing or has an
    /// unsupported type, mirroring the vector-parameter paths.
    pub(super) fn resolve_stmt_where_params(
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<crate::velesql::SelectStatement>> {
        let Some(cond) = stmt.where_clause.as_ref() else {
            return Ok(None);
        };
        let resolved = Self::resolve_condition_params(cond, params)?;
        let mut resolved_stmt = stmt.clone();
        resolved_stmt.where_clause = Some(resolved);
        Ok(Some(resolved_stmt))
    }

    /// Returns a copy of `query` with scalar WHERE parameter placeholders
    /// resolved, or `None` when the query has no WHERE clause.
    ///
    /// Convenience wrapper around [`Self::resolve_stmt_where_params`] for
    /// callers that thread a whole [`Query`](crate::velesql::Query) through
    /// their pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error when a referenced parameter is missing or has an
    /// unsupported type.
    pub(super) fn resolve_query_where_params(
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<crate::velesql::Query>> {
        Ok(
            Self::resolve_stmt_where_params(&query.select, params)?.map(|select| {
                crate::velesql::Query {
                    select,
                    ..query.clone()
                }
            }),
        )
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
        let mut fused_search = None;

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
            fused_search = self.extract_fused_vectors(cond, params)?;

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
            fused_search,
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
        let Some(group_by) = stmt.group_by.as_ref() else {
            return results.to_vec();
        };
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
            .inspect_err(|_| self.runtime.guard_rails.circuit_breaker.record_failure())?;
        ctx.check_cardinality(result_count)
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.runtime.guard_rails.circuit_breaker.record_failure())?;
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

        // Thread the live indexed-field set and (for MATCH) the real graph
        // CollectionStats so the plan emits IndexLookup / a MatchTraversal node
        // with a calibrated strategy instead of a bare TableScan (backlog #14).
        // Cache fields are unavailable at the Collection level and remain None.
        let indexed = self.indexed_field_names();
        let match_stats = self.compute_match_collection_stats();
        let plan = QueryPlan::from_query_with_all_stats(query, &indexed, None, Some(&match_stats));

        let start = std::time::Instant::now();
        let (results, nodes, edges) = self.execute_query_counted(query, params)?;
        let stats = ActualStats::from_counted(results.len() as u64, start.elapsed(), nodes, edges);
        let node_stats = build_leaf_node_stats(&plan.root, stats.actual_rows, stats.actual_time_ms);
        let mut output = ExplainOutput::with_stats(plan, stats, node_stats);

        // Issue #469 Phase 2: attach EMA-calibrated ms_per_cost_unit if the
        // feedback loop is warm (≥ MIN_SAMPLES observations on this collection).
        if let Some(ms_per_unit) = self.query.query_planner.adjusted_ms_per_cost_unit() {
            output = output.with_feedback_calibration(
                ms_per_unit,
                self.query.query_planner.cbo_sample_count(),
            );
        }

        Ok(output)
    }
}
