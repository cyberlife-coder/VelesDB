//! VelesQL query execution for Collection.
//!
//! This module orchestrates query execution by combining:
//! - Query validation (`validation.rs`)
//! - Condition extraction (`extraction.rs`)
//! - ORDER BY processing (`ordering.rs`)
//!
//! # Cost-based optimization wiring (issue #467)
//!
//! `compute_cbo_strategy` in `select_dispatch.rs` routes between two planner
//! entry points depending on query shape:
//!
//! - Queries carrying `ORDER BY similarity()` go through
//!   [`crate::velesql::QueryPlanner::choose_hybrid_strategy`], which forces
//!   `VectorFirst` to preserve HNSW's natural similarity ordering and applies
//!   a selectivity-aware over-fetch factor.
//! - All other SELECT queries go through
//!   [`crate::velesql::QueryPlanner::choose_strategy_with_cbo_and_overfetch`],
//!   which derives I/O / CPU weights from calibrated `OperationCostFactors`
//!   (or defaults when the collection was never analyzed).
//!
//! Both entry points share the same return shape
//! `(ExecutionStrategy, over_fetch: usize)` consumed by
//! `dispatch_vector_query` in `execution_paths.rs`. The deeper
//! multi-candidate `PlanGenerator` enumeration remains open
//! (see `collection/query_cost/plan_generator.rs`); it is reserved for
//! a future expansion that would supersede the current two-path routing
//! with full cost-based enumeration.

#![allow(clippy::uninlined_format_args)] // Prefer readability in query error paths.
#![allow(clippy::implicit_hasher)] // HashSet hasher genericity adds noise for internal APIs.

mod aggregation;
mod bounded_top_k;
#[cfg(test)]
mod component_scores_tests;
pub(crate) use crate::velesql::condition_tree;
mod early_return;
mod execution_paths;
mod extraction;
#[cfg(test)]
mod extraction_tests;
mod fetch_window;
mod fused_dispatch;
mod graph_prefilter;
mod hybrid_sparse;
#[cfg(test)]
mod hybrid_sparse_tests;
pub mod join;
#[cfg(test)]
mod join_tests;
#[cfg(test)]
mod let_execution_tests;
mod match_dispatch;
pub mod match_exec;
#[cfg(test)]
mod match_exec_tests;
pub mod match_metrics;
#[cfg(test)]
mod match_metrics_tests;
#[cfg(test)]
mod match_order_by_validation_parity_tests;
#[cfg(test)]
mod match_row_bound_tests;
#[cfg(test)]
mod match_row_universe_tests;
mod metadata_query;
mod multi_vector;
#[cfg(test)]
mod multi_vector_tests;
#[cfg(test)]
mod near_disjunction_tests;
mod options;
mod ordered_index_scan;
mod ordered_scan;
mod ordering;
#[cfg(test)]
mod ordering_tests;
#[cfg(test)]
mod ordering_window_tests;
pub mod parallel_traversal;
#[cfg(test)]
mod parallel_traversal_tests;
pub mod projection;
pub mod pushdown;
#[cfg(test)]
mod pushdown_tests;
mod query_pipeline;
pub(crate) mod row_universe;
#[cfg(test)]
mod scan_cap_observability_tests;
mod select_dispatch;
mod similarity_filter;
mod similarity_scan;
mod sparse_dispatch;
pub(crate) mod text_match;
#[cfg(test)]
mod text_match_tests;
mod validation;
pub(crate) mod vector_group_by;
mod where_eval;
#[cfg(test)]
mod with_options_tests;

// Re-export for potential external use
#[allow(unused_imports)]
pub use ordering::compare_json_values;
// Re-export join functions for future integration with execute_query
#[allow(unused_imports)]
pub use join::{execute_join, JoinedResult, JOIN_ROW_CEILING};

// Re-export types from options.rs so sibling submodules can use `super::*`.
pub(crate) use options::QuerySearchOptions;

/// Outcome of a counted (EXPLAIN ANALYZE) execution: the results plus the
/// execution facts the plain path discards — graph-traversal counters and
/// the filter strategy the executor actually ran.
pub(crate) struct CountedExecution {
    /// Query results, as `execute_query` would return them.
    pub(crate) results: Vec<crate::point::SearchResult>,
    /// Approximate MATCH traversal node count (0 for non-MATCH).
    pub(crate) nodes_visited: u64,
    /// Approximate MATCH traversal edge count (0 for non-MATCH).
    pub(crate) edges_traversed: u64,
    /// Strategy the filtered vector-search dispatch recorded, if any.
    pub(crate) executed_filter_strategy: Option<crate::velesql::FilterStrategy>,
}
pub(crate) use options::MAX_LIMIT;
pub(in crate::collection::search::query) use options::{
    ExtractedComponents, QueryFinalizationContext,
};

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use std::collections::HashSet;

/// Bundles the non-query/params arguments for
/// [`Collection::dispatch_and_finalize`] to stay within the parameter limit.
struct DispatchFinalizeArgs<'a> {
    extracted: &'a ExtractedComponents,
    search_opts: &'a QuerySearchOptions,
    limit: usize,
    fetch_limit: usize,
    is_vgb: bool,
    ctx: &'a crate::guardrails::QueryContext,
    started_at: std::time::Instant,
}

impl DispatchFinalizeArgs<'_> {
    fn finalization<'a>(
        &'a self,
        stmt: &'a crate::velesql::SelectStatement,
        params: &'a std::collections::HashMap<String, serde_json::Value>,
        let_bindings: &'a [crate::velesql::LetBinding],
    ) -> QueryFinalizationContext<'a> {
        QueryFinalizationContext {
            stmt,
            params,
            limit: self.limit,
            extracted: self.extracted,
            ctx: self.ctx,
            started_at: self.started_at,
            let_bindings,
        }
    }
}

impl Collection {
    /// Executes a `VelesQL` query on this collection with the `"default"` client id.
    ///
    /// This method unifies vector search, text search, and metadata filtering
    /// into a single interface. Compound queries (`UNION`, `INTERSECT`, `EXCEPT`)
    /// are resolved here before delegation. For per-client rate limiting use
    /// [`execute_query_with_client`](Self::execute_query_with_client).
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed (e.g., missing parameters).
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        if query.compound.is_none() {
            return self.execute_query_with_client(query, params, "default");
        }
        // The compound's own rules (an aggregation is no operand, `V019`)
        // before its operands run, each validated as the SELECT it is.
        crate::velesql::QueryValidator::validate(query)?;
        // Compound semantics are velesql's (`velesql::execute_compound`),
        // shared with the `Database` executor and WASM; each operand runs here.
        self.execute_compound_read(query, params, &mut |operand| {
            let inputs = crate::velesql::compound_rank_inputs(query, &operand.select.columns);
            self.execute_set_operand(operand, params, inputs)
        })
    }

    /// Runs a native compound using its first collection's ordering policy.
    pub(crate) fn execute_compound_read(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        run_operand: &mut dyn FnMut(&crate::velesql::Query) -> Result<Vec<SearchResult>>,
    ) -> Result<Vec<SearchResult>> {
        let higher_is_better = self.storage.config.read().metric.higher_is_better();
        crate::velesql::execute_compound(
            query,
            higher_is_better,
            u64::try_from(MAX_LIMIT).unwrap_or(u64::MAX),
            run_operand,
            |rows, order_by| self.apply_order_by(rows, order_by, params),
        )
    }

    /// Runs an operand and retains only the rank inputs its compound will order.
    pub(crate) fn execute_set_operand(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        inputs: Vec<String>,
    ) -> Result<Vec<SearchResult>> {
        let mut ctx = self.prepare_query_context(query, "default")?;
        ctx.window_rank_inputs = inputs;
        self.execute_in_query_context(query, params, &ctx)
            .map(|(rows, _)| rows)
    }

    /// Executes a `VelesQL` query with a specific client identifier for per-client rate limiting.
    ///
    /// Each distinct `client_id` maintains an independent token bucket, so one
    /// busy client cannot exhaust the quota of another.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed or a guard-rail fires.
    pub fn execute_query_with_client(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        client_id: &str,
    ) -> Result<Vec<SearchResult>> {
        self.execute_query_traced(query, params, client_id)
            .map(|(results, _)| results)
    }

    /// Executes a single (non-compound) query, storing the executor's ran
    /// filter strategy into the caller's slot. Used by the Database-level
    /// EXPLAIN ANALYZE counted path, whose probe travels through the JOIN
    /// resolution machinery down to this base-collection call.
    pub(crate) fn execute_query_probed(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        slot: &crate::guardrails::ExecutedStrategyCell,
    ) -> Result<Vec<SearchResult>> {
        let (results, strategy) = self.execute_query_traced(query, params, "default")?;
        if let Some(strategy) = strategy {
            slot.record(strategy);
        }
        Ok(results)
    }

    /// Like [`execute_query_with_client`](Self::execute_query_with_client),
    /// additionally returning the filter strategy the executor actually ran
    /// (recorded through the query context's probe — `None` when the query
    /// did not go through the filtered vector-search dispatch, e.g. MATCH).
    /// Consumed by EXPLAIN ANALYZE at both the Collection and Database level.
    pub(crate) fn execute_query_traced(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        client_id: &str,
    ) -> Result<(Vec<SearchResult>, Option<crate::velesql::FilterStrategy>)> {
        let ctx = self.prepare_query_context(query, client_id)?;
        self.execute_in_query_context(query, params, &ctx)
    }

    /// Executes an already validated read under its statement's deadline and guard rails.
    pub(crate) fn execute_in_query_context(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<(Vec<SearchResult>, Option<crate::velesql::FilterStrategy>)> {
        // MATCH queries take a completely separate path (no extraction needed).
        if let Some(results) =
            self.try_dispatch_match(query, params, ctx, &match_exec::MatchSources::default())?
        {
            return Ok((results, None));
        }

        let resolved_query = self.bind_select(query, params)?;
        let query = resolved_query.as_ref().unwrap_or(query);

        // Phase 2-3: SELECT extraction, early-return, dispatch, and finalization.
        let results = self.execute_select_pipeline(query, params, ctx)?;
        Ok((results, ctx.executed_filter_strategy()))
    }

    /// The statement a SELECT runs, checked once before any row is read: a
    /// query vector this collection cannot score fails it here, whatever
    /// plan, operator or row set would follow; its WHERE's shape is validated
    /// as written — the one validation, which extraction relies on
    /// ([`Self::validate_similarity_query_structure`]); then its scalar
    /// parameters are resolved, so every downstream conversion to a payload
    /// Filter sees bound values and a missing parameter errors here instead
    /// of silently degrading to NULL, and its text MATCH leaves are bound.
    /// `None` when there is no WHERE to bind.
    fn bind_select(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<crate::velesql::Query>> {
        self.check_select_query_vectors(&query.select, params)?;
        if let Some(cond) = query.select.where_clause.as_ref() {
            Self::validate_similarity_query_structure(cond)?;
        }
        self.bind_query_where(query, params, text_match::WhereReader::Select)
    }

    /// Runs the full SELECT pipeline: extraction, early-return check, dispatch,
    /// and post-processing.
    ///
    /// Called only after MATCH dispatch has been ruled out. Extracts query components
    /// once and shares them across early-return paths and the main dispatch.
    fn execute_select_pipeline(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let stmt = &query.select;
        let started_at = ctx.select_started_at();
        let (limit, fetch_limit) = Self::compute_fetch_limit(stmt);
        let mut extracted = Self::extract_query_components(stmt, params)?;

        // A covering secondary index can supply the finished ORDER BY page.
        // The scan checks the filter and sort shape before applying OFFSET/LIMIT.
        if let Some(results) = self.try_ordered_index_scan(query, stmt, &extracted, ctx)? {
            return Ok(results);
        }

        // The fetch stops at LIMIT + OFFSET only when the rows it stops at
        // are the window's (`fetch_window.rs`); an ORDER BY it does not rank
        // by takes the ordered scan, which ranks every admitted row.
        let mut search_opts = QuerySearchOptions::from_with_clause(stmt.with_clause.as_ref())?
            .with_fusion(stmt.fusion_clause.clone())
            .with_executed_strategy_probe(ctx);
        let window =
            self.select_fetch_window(stmt, &mut extracted, params, fetch_limit, &mut search_opts);
        let args = DispatchFinalizeArgs {
            extracted: &extracted,
            search_opts: &search_opts,
            limit,
            fetch_limit,
            is_vgb: stmt.is_vector_group_by(),
            ctx,
            started_at,
        };
        self.execute_fetch_window(query, params, window, args)
    }

    /// Runs the selected candidate window and finalizes its rows.
    fn execute_fetch_window(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        window: fetch_window::FetchWindow,
        mut args: DispatchFinalizeArgs<'_>,
    ) -> Result<Vec<SearchResult>> {
        let stmt = &query.select;
        args.fetch_limit = match window {
            fetch_window::FetchWindow::Bounded(rows) => rows,
            fetch_window::FetchWindow::Exhaustive => MAX_LIMIT,
            fetch_window::FetchWindow::OrderedScan { pq_scores } => {
                let context = args.finalization(stmt, params, &query.let_bindings);
                return self.execute_ordered_scan(&context, args.search_opts, pq_scores);
            }
        };

        // Early-return paths or LET-binding guard for special query shapes.
        if let Some(results) = self.try_early_return_or_guard_let(
            query,
            stmt,
            params,
            args.extracted,
            args.fetch_limit,
            args.ctx,
        )? {
            return Ok(results);
        }

        self.dispatch_and_finalize::<MAX_LIMIT>(query, params, &args)
    }

    /// Runs the main dispatch, optional vector GROUP BY, and finalization
    /// (DISTINCT / window / ORDER BY / OFFSET / LIMIT / LET injection).
    ///
    /// Split out of [`Self::execute_select_pipeline`] so each stays within the
    /// cyclomatic-complexity budget.
    fn dispatch_and_finalize<const CEILING: usize>(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        args: &DispatchFinalizeArgs<'_>,
    ) -> Result<Vec<SearchResult>> {
        let stmt = &query.select;
        let mut most_fetched = 0;
        let mut results = fetch_window::fetch_past_ties(stmt, args.fetch_limit, |rows| {
            let (fetched, results) =
                self.dispatch_main_select(stmt, params, args.extracted, rows, args.search_opts)?;
            most_fetched = most_fetched.max(fetched);
            Ok(results)
        })?;
        self.warn_if_window_capped_at(stmt, most_fetched, CEILING);

        // Vector GROUP BY post-processing: group results by parent field
        // before ORDER BY / LIMIT / OFFSET are applied, which then sort the
        // groups on the payload fields their aggregates landed in.
        let stmt = if args.is_vgb {
            results = self.apply_vector_group_by(stmt, &results, params)?;
            stmt.order_by_aggregates_as_fields()?
        } else {
            std::borrow::Cow::Borrowed(stmt)
        };

        self.finalize_query_results(
            &mut results,
            &args.finalization(&stmt, params, &query.let_bindings),
        )?;
        // The ef_search cost estimate belongs to vector dispatch. The separate
        // ordered scan shares finalization and records its latency there.
        if args.extracted.vector_search.is_some() {
            let ef_search = stmt
                .with_clause
                .as_ref()
                .and_then(crate::velesql::WithClause::get_ef_search)
                .unwrap_or(100);
            self.query.query_planner.record_cbo_feedback(
                self.vector_count(),
                ef_search,
                args.started_at.elapsed().as_secs_f64() * 1000.0,
            );
        }
        Ok(results)
    }

    /// Parses and executes a VelesQL query string, using the collection-level parse cache (P1-A).
    ///
    /// Equivalent to calling `Parser::parse(sql)` followed by `execute_query()`, but caches
    /// parsed ASTs so repeated identical queries avoid re-parsing overhead.
    ///
    /// # Arguments
    ///
    /// * `sql` - Raw VelesQL query string
    /// * `params` - Query parameters for resolving placeholders (e.g., `$v`)
    ///
    /// # Errors
    ///
    /// Returns a parse error if `sql` is invalid, or an execution error if the query fails.
    pub fn execute_query_str(
        &self,
        sql: &str,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let query = self.query.query_cache.parse(sql)?;
        self.execute_query(&query, params)
    }

    // NOTE: try_dispatch_match, compute_fetch_limit, try_early_return_or_guard_let,
    // validate_let_binding_support, prepare_query_context, finalize_query_results,
    // extract_query_components, apply_vector_group_by,
    // check_guardrails_and_record, explain_analyze_query
    // → moved to query_pipeline.rs (NLOC/file reduction)

    // NOTE: try_early_return_path, try_not_similarity_or_union, execute_early_return_query
    // moved to early_return.rs (NLOC/CC resolution batch 3)

    // NOTE: dispatch_sparse_query, execute_sparse_or_hybrid, filter_by_graph_predicates,
    // finalize_ranked_results, resolve_fusion_strategy moved to sparse_dispatch.rs (T3-3)

    // NOTE: compute_cbo_strategy, dispatch_main_select, dispatch_match_query,
    // analyze_join_pushdown, apply_select_postprocessing moved to select_dispatch.rs

    // NOTE: apply_distinct and compute_distinct_key moved to distinct.rs
    // (EPIC-061/US-003 refactoring), then to velesql/distinct_evaluator.rs
}
