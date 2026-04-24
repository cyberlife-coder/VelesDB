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
//!   [`QueryPlanner::choose_hybrid_strategy`], which forces `VectorFirst`
//!   to preserve HNSW's natural similarity ordering and applies a
//!   selectivity-aware over-fetch factor.
//! - All other SELECT queries go through
//!   [`QueryPlanner::choose_strategy_with_cbo_and_overfetch`], which
//!   derives I/O / CPU weights from calibrated `OperationCostFactors`
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
#[cfg(test)]
mod component_scores_tests;
pub(crate) mod condition_tree;
mod distinct;
#[cfg(test)]
mod distinct_tests;
mod early_return;
mod execution_paths;
mod extraction;
#[cfg(test)]
mod extraction_tests;
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
pub mod match_planner;
#[cfg(test)]
mod match_planner_tests;
mod metadata_query;
mod multi_vector;
#[cfg(test)]
mod multi_vector_tests;
mod options;
mod ordering;
#[cfg(test)]
mod ordering_tests;
pub mod parallel_traversal;
#[cfg(test)]
mod parallel_traversal_tests;
pub mod projection;
pub mod pushdown;
#[cfg(test)]
mod pushdown_tests;
mod query_pipeline;
pub mod score_fusion;
#[cfg(test)]
mod score_fusion_tests;
mod select_dispatch;
pub(crate) mod set_operations;
mod similarity_filter;
mod sparse_dispatch;
mod union_query;
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
pub use join::{execute_join, JoinedResult};

// Re-export types from options.rs so sibling submodules can use `super::*`.
pub(crate) use options::QuerySearchOptions;
pub(in crate::collection::search::query) use options::{
    ExtractedComponents, QueryFinalizationContext, MAX_LIMIT,
};

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use std::collections::HashSet;

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
        // EPIC-040 US-006: For compound queries, execute each operand without the
        // outer LIMIT so the set operation sees the full result sets.  The final
        // LIMIT is applied once on the merged output (SQL-standard behaviour).
        // Use MAX_LIMIT (not None) to avoid the default-10 cap in execute_query_with_client.
        let compound_limit = Some(u64::try_from(MAX_LIMIT).unwrap_or(u64::MAX));
        let left_results = if query.compound.is_some() {
            let mut left_query = query.clone();
            left_query.select.limit = compound_limit;
            left_query.select.offset = None; // OFFSET applies to combined result, not operands.
            left_query.compound = None;
            self.execute_query_with_client(&left_query, params, "default")?
        } else {
            return self.execute_query_with_client(query, params, "default");
        };

        // compound is guaranteed Some here (non-compound returns above).
        if let Some(ref compound) = query.compound {
            let mut accumulated = left_results;
            for (operator, right_select) in &compound.operations {
                let mut right_query = crate::velesql::Query::new_select(right_select.clone());
                right_query.select.limit = compound_limit;
                let right_results =
                    self.execute_query_with_client(&right_query, params, "default")?;
                accumulated =
                    set_operations::apply_set_operation(accumulated, right_results, *operator);
            }
            // SQL-standard: OFFSET then LIMIT on the combined result.
            if let Some(offset) = query.select.offset {
                let skip = usize::try_from(offset).unwrap_or(usize::MAX);
                accumulated = accumulated.into_iter().skip(skip).collect();
            }
            if let Some(limit) = query.select.limit {
                accumulated.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            }
            return Ok(accumulated);
        }

        Ok(left_results)
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
        // Phase 1: Pre-checks and context setup.
        let ctx = self.prepare_query_context(query, client_id)?;

        // MATCH queries take a completely separate path (no extraction needed).
        if let Some(results) = self.try_dispatch_match(query, params, &ctx)? {
            return Ok(results);
        }

        // Phase 2-3: SELECT extraction, early-return, dispatch, and finalization.
        self.execute_select_pipeline(query, params, &ctx)
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
        let (limit, fetch_limit) = Self::compute_fetch_limit(stmt);
        let extracted = self.extract_query_components(stmt, params)?;

        // When vector GROUP BY is active, fetch more results from vector search
        // so grouping has enough chunks to work with.
        let is_vgb = vector_group_by::is_vector_group_by_query(stmt);
        let effective_fetch_limit = if is_vgb { MAX_LIMIT } else { fetch_limit };

        // Early-return paths or LET-binding guard for special query shapes.
        if let Some(results) = self.try_early_return_or_guard_let(
            query,
            stmt,
            params,
            &extracted,
            effective_fetch_limit,
            ctx,
        )? {
            return Ok(results);
        }

        // Main dispatch + post-processing.
        let mut results =
            self.dispatch_main_select(stmt, params, &extracted, effective_fetch_limit, ctx)?;

        // Vector GROUP BY post-processing: group results by parent field
        // before ORDER BY / LIMIT / OFFSET are applied.
        if is_vgb {
            results = self.apply_vector_group_by(stmt, &results);
        }

        self.finalize_query_results(
            &mut results,
            &QueryFinalizationContext {
                stmt,
                params,
                limit,
                extracted: &extracted,
                ctx,
                let_bindings: &query.let_bindings,
            },
        )?;
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
        let query = self
            .query_cache
            .parse(sql)
            .map_err(|e| crate::error::Error::Query(e.to_string()))?;
        self.execute_query(&query, params)
    }

    // NOTE: try_dispatch_match, compute_fetch_limit, try_early_return_or_guard_let,
    // validate_let_binding_support, prepare_query_context, finalize_query_results,
    // extract_query_components, apply_vector_group_by, extract_aggregations,
    // check_guardrails_and_record, explain_analyze_query
    // → moved to query_pipeline.rs (NLOC/file reduction)

    // NOTE: try_early_return_path, try_not_similarity_or_union, execute_early_return_query
    // moved to early_return.rs (NLOC/CC resolution batch 3)

    // NOTE: dispatch_sparse_query, execute_sparse_or_hybrid, filter_by_graph_predicates,
    // finalize_sparse_results, resolve_fusion_strategy moved to sparse_dispatch.rs (T3-3)

    // NOTE: compute_cbo_strategy, dispatch_main_select, dispatch_match_query,
    // analyze_join_pushdown, apply_select_postprocessing moved to select_dispatch.rs

    // NOTE: apply_distinct and compute_distinct_key moved to distinct.rs
    // (EPIC-061/US-003 refactoring)

    // NOTE: filter_by_similarity, execute_not_similarity_query, extract_not_similarity_condition,
    // execute_scan_query moved to similarity_filter.rs (Plan 04-04)

    // NOTE: execute_union_query, matches_metadata_filter, split_or_condition_with_outer_filter
    // moved to union_query.rs (Plan 04-04)
}
