//! VelesQL query execution for Collection.
//!
//! This module orchestrates query execution by combining:
//! - Query validation (`validation.rs`)
//! - Condition extraction (`extraction.rs`)
//! - ORDER BY processing (`ordering.rs`)
//!
//! # Future Enhancement: HybridExecutionPlan Integration
//!
//! The `HybridExecutionPlan` and `choose_hybrid_strategy()` in `planner.rs`
//! are ready for integration to optimize query execution based on:
//! - Query pattern (ORDER BY similarity, filters, etc.)
//! - Runtime statistics (latency, selectivity)
//! - Over-fetch factor for filtered queries
//!
//! Future: Integrate `QueryPlanner::choose_hybrid_strategy()` into `execute_query()`
//! to leverage cost-based optimization for complex queries.

#![allow(clippy::uninlined_format_args)] // Prefer readability in query error paths.
#![allow(clippy::implicit_hasher)] // HashSet hasher genericity adds noise for internal APIs.

mod aggregation;
mod distinct;
#[cfg(test)]
mod distinct_tests;
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
pub mod match_exec;
#[cfg(test)]
mod match_exec_tests;
pub mod match_metrics;
#[cfg(test)]
mod match_metrics_tests;
pub mod match_planner;
#[cfg(test)]
mod match_planner_tests;
mod multi_vector;
#[cfg(test)]
mod multi_vector_tests;
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
pub mod score_fusion;
#[cfg(test)]
mod score_fusion_tests;
mod select_dispatch;
pub(crate) mod set_operations;
mod similarity_filter;
mod sparse_dispatch;
mod union_query;
mod validation;
mod where_eval;

// Re-export for potential external use
#[allow(unused_imports)]
pub use ordering::compare_json_values;
// Re-export join functions for future integration with execute_query
#[allow(unused_imports)]
pub use join::{execute_join, JoinedResult};

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use std::collections::HashSet;

/// Maximum allowed LIMIT value to prevent overflow in over-fetch calculations.
const MAX_LIMIT: usize = 100_000;

/// Context for early-return query paths (NOT-similarity, union).
struct EarlyReturnCtx<'a> {
    stmt: &'a crate::velesql::SelectStatement,
    params: &'a std::collections::HashMap<String, serde_json::Value>,
    cond: &'a crate::velesql::Condition,
    has_graph_predicates: bool,
    limit: usize,
    ctx: &'a crate::guardrails::QueryContext,
}

/// Extracted query components from the WHERE clause.
struct ExtractedComponents {
    vector_search: Option<Vec<f32>>,
    similarity_conditions: Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)>,
    filter_condition: Option<crate::velesql::Condition>,
    graph_match_predicates: Vec<crate::velesql::GraphMatchPredicate>,
    sparse_vector_search: Option<crate::velesql::SparseVectorSearch>,
    is_union_query: bool,
    is_not_similarity_query: bool,
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
        // EPIC-040 US-006: For compound queries, execute each operand without the
        // outer LIMIT so the set operation sees the full result sets.  The final
        // LIMIT is applied once on the merged output (SQL-standard behaviour).
        // Use MAX_LIMIT (not None) to avoid the default-10 cap in execute_query_with_client.
        let compound_limit = Some(u64::try_from(MAX_LIMIT).unwrap_or(u64::MAX));
        let left_results = if query.compound.is_some() {
            let mut left_query = query.clone();
            left_query.select.limit = compound_limit;
            left_query.compound = None;
            self.execute_query_with_client(&left_query, params, "default")?
        } else {
            return self.execute_query_with_client(query, params, "default");
        };

        // compound is guaranteed Some here (non-compound returns above).
        if let Some(ref compound) = query.compound {
            let mut right_query = crate::velesql::Query::new_select(*compound.right.clone());
            right_query.select.limit = compound_limit;
            let right_results = self.execute_query_with_client(&right_query, params, "default")?;
            let mut merged =
                set_operations::apply_set_operation(left_results, right_results, compound.operator);
            // SQL-standard: LIMIT from the left (outer) SELECT applies to the final result.
            if let Some(limit) = query.select.limit {
                merged.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            }
            return Ok(merged);
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
        // Guard-rail pre-checks: circuit breaker + rate limiting (EPIC-048).
        self.guard_rails
            .pre_check(client_id)
            .map_err(crate::error::Error::from)?;

        // Create per-query execution context for timeout + cardinality tracking.
        let ctx = self.guard_rails.create_context();

        crate::velesql::QueryValidator::validate(query)
            .map_err(|e| crate::error::Error::Query(e.to_string()))?;

        // Unified VelesQL dispatch: allow Collection::execute_query() to run top-level MATCH queries.
        if let Some(match_clause) = query.match_clause.as_ref() {
            return self.dispatch_match_query(match_clause, params, &ctx);
        }

        let stmt = &query.select;
        let limit = usize::try_from(stmt.limit.unwrap_or(10))
            .unwrap_or(MAX_LIMIT)
            .min(MAX_LIMIT);

        let extracted = self.extract_query_components(stmt, params)?;

        // Early-return paths for special query shapes.
        if let Some(results) = self.try_early_return_path(stmt, params, &extracted, limit, &ctx)? {
            return Ok(results);
        }

        // Main vector/similarity/metadata dispatch path.
        let mut results = self.dispatch_main_select(stmt, params, &extracted, limit, &ctx)?;

        // JOIN pushdown analysis (EPIC-031 US-006).
        self.analyze_join_pushdown(stmt);

        // Final guard-rail checks (EPIC-048).
        self.check_guardrails_and_record(&ctx, results.len())?;

        // Post-processing: DISTINCT, ORDER BY, LIMIT.
        results = self.apply_select_postprocessing(stmt, results, params, limit)?;

        // Update QueryPlanner adaptive stats for vector/SELECT queries (Fix #8).
        if extracted.vector_search.is_some() {
            // Reason: u128->u64 cast; query durations < u64::MAX µs (~585 millennia)
            #[allow(clippy::cast_possible_truncation)]
            let vector_latency_us = ctx.elapsed().as_micros() as u64;
            self.query_planner
                .stats()
                .update_vector_latency(vector_latency_us);
        }
        self.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }

    /// Extracts all query components from the SELECT statement's WHERE clause.
    fn extract_query_components(
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

    /// Attempts early-return paths: NOT-similarity, union, and sparse queries.
    ///
    /// Returns `Ok(Some(results))` if an early path was taken, `Ok(None)` otherwise.
    fn try_early_return_path(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Option<Vec<SearchResult>>> {
        if let Some(results) =
            self.try_not_similarity_or_union(stmt, params, extracted, limit, ctx)?
        {
            return Ok(Some(results));
        }

        // Phase 5: Sparse-only or hybrid dense+sparse execution.
        if let Some(ref svs) = extracted.sparse_vector_search {
            let results = self.dispatch_sparse_query(stmt, params, extracted, svs, limit, ctx)?;
            return Ok(Some(results));
        }

        Ok(None)
    }

    /// Handles NOT-similarity and union early-return paths.
    fn try_not_similarity_or_union(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Option<Vec<SearchResult>>> {
        let cond = match stmt.where_clause.as_ref() {
            Some(c) if extracted.is_not_similarity_query || extracted.is_union_query => c,
            _ => return Ok(None),
        };

        let has_graph_predicates = !extracted.graph_match_predicates.is_empty();
        let execution_limit = if has_graph_predicates {
            MAX_LIMIT
        } else {
            limit
        };

        let early_ctx = EarlyReturnCtx {
            stmt,
            params,
            cond,
            has_graph_predicates,
            limit,
            ctx,
        };

        // EPIC-044 US-003: NOT similarity() requires full scan
        if extracted.is_not_similarity_query {
            let results = self.execute_early_return_query(
                |s| s.execute_not_similarity_query(cond, params, execution_limit),
                &early_ctx,
            )?;
            return Ok(Some(results));
        }

        // EPIC-044 US-002: Union mode for similarity() OR metadata
        let results = self.execute_early_return_query(
            |s| s.execute_union_query(cond, params, execution_limit),
            &early_ctx,
        )?;
        Ok(Some(results))
    }

    /// Executes an early-return query path with guard-rail checks and post-processing.
    fn execute_early_return_query(
        &self,
        execute_fn: impl FnOnce(&Self) -> Result<Vec<SearchResult>>,
        early: &EarlyReturnCtx<'_>,
    ) -> Result<Vec<SearchResult>> {
        let mut results =
            execute_fn(self).inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        if early.has_graph_predicates {
            results = self
                .apply_where_condition_to_results(
                    results,
                    early.cond,
                    early.params,
                    &early.stmt.from_alias,
                )
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        }
        if let Some(ref order_by) = early.stmt.order_by {
            self.apply_order_by(&mut results, order_by, early.params)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        }
        results.truncate(early.limit);
        self.check_guardrails_and_record(early.ctx, results.len())?;
        self.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }

    // NOTE: dispatch_sparse_query, execute_sparse_or_hybrid, filter_by_graph_predicates,
    // finalize_sparse_results, resolve_fusion_strategy moved to sparse_dispatch.rs (T3-3)

    // NOTE: compute_cbo_strategy, dispatch_main_select, dispatch_match_query,
    // analyze_join_pushdown, apply_select_postprocessing moved to select_dispatch.rs

    /// Checks timeout and cardinality guard-rails, recording failure on violation.
    fn check_guardrails_and_record(
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

    // NOTE: apply_distinct and compute_distinct_key moved to distinct.rs
    // (EPIC-061/US-003 refactoring)

    // NOTE: filter_by_similarity, execute_not_similarity_query, extract_not_similarity_condition,
    // execute_scan_query moved to similarity_filter.rs (Plan 04-04)

    // NOTE: execute_union_query, matches_metadata_filter, split_or_condition_with_outer_filter
    // moved to union_query.rs (Plan 04-04)
}
