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
#[cfg(test)]
mod with_options_tests;

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

/// Query-time search options extracted from the WITH clause.
///
/// Consolidates `mode`, `ef_search`, `rerank`, and `fusion_clause` into a single
/// struct that flows through all dispatch paths. When no WITH clause is present,
/// all fields are `None` and the default behavior is preserved.
#[derive(Debug, Clone, Default)]
pub(crate) struct QuerySearchOptions {
    /// Search quality profile parsed from `WITH (mode='...')`.
    pub quality: Option<crate::SearchQuality>,
    /// Explicit ef_search override from `WITH (ef_search=N)`.
    pub ef_search: Option<usize>,
    /// Force reranking on (`true`) or off (`false`) from `WITH (rerank=...)`.
    pub force_rerank: Option<bool>,
    /// Fusion clause from `USING FUSION (...)`.
    pub fusion_clause: Option<crate::velesql::FusionClause>,
}

impl QuerySearchOptions {
    /// Extracts search options from an optional WITH clause and fusion clause.
    ///
    /// Maps `mode` string to [`SearchQuality`](crate::SearchQuality) using the
    /// same parsing logic as `mode_to_search_quality()`. Invalid mode strings
    /// are silently ignored (quality remains `None`).
    #[must_use]
    pub(crate) fn from_with_clause(with: Option<&crate::velesql::WithClause>) -> Self {
        let Some(with) = with else {
            return Self::default();
        };

        let quality = with.get_mode().and_then(parse_mode_to_quality);

        let ef_search = with.get_ef_search();
        let force_rerank = with.get_rerank();

        Self {
            quality,
            ef_search,
            force_rerank,
            fusion_clause: None,
        }
    }

    /// Creates options with a fusion clause attached.
    #[must_use]
    pub(crate) fn with_fusion(mut self, fusion: Option<crate::velesql::FusionClause>) -> Self {
        self.fusion_clause = fusion;
        self
    }

    /// Returns `true` when any quality-related override is set.
    #[must_use]
    pub(crate) fn has_quality_overrides(&self) -> bool {
        self.quality.is_some() || self.ef_search.is_some() || self.force_rerank.is_some()
    }
}

/// Maps a mode string from `WITH (mode='...')` to a [`SearchQuality`](crate::SearchQuality).
///
/// Delegates to [`crate::api_types::mode_to_search_quality`] which also handles
/// advanced modes (`custom:<ef>`, `adaptive:<min>:<max>`).
#[cfg(feature = "persistence")]
fn parse_mode_to_quality(mode: &str) -> Option<crate::SearchQuality> {
    crate::api_types::mode_to_search_quality(mode)
}

// EarlyReturnCtx is in early_return.rs

/// Extracted query components from the WHERE clause.
pub(super) struct ExtractedComponents {
    pub(super) vector_search: Option<Vec<f32>>,
    pub(super) similarity_conditions: Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)>,
    pub(super) filter_condition: Option<crate::velesql::Condition>,
    pub(super) graph_match_predicates: Vec<crate::velesql::GraphMatchPredicate>,
    pub(super) sparse_vector_search: Option<crate::velesql::SparseVectorSearch>,
    pub(super) is_union_query: bool,
    pub(super) is_not_similarity_query: bool,
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

        // Early-return paths or LET-binding guard for special query shapes.
        if let Some(results) =
            self.try_early_return_or_guard_let(query, stmt, params, &extracted, fetch_limit, ctx)?
        {
            return Ok(results);
        }

        // Main dispatch + post-processing.
        let mut results = self.dispatch_main_select(stmt, params, &extracted, fetch_limit, ctx)?;
        self.finalize_query_results(
            stmt,
            &mut results,
            params,
            limit,
            &extracted,
            ctx,
            &query.let_bindings,
        )?;
        Ok(results)
    }

    /// Dispatches a MATCH query if the query contains a `match_clause`.
    ///
    /// Returns `Ok(Some(results))` when the MATCH path was taken, `Ok(None)` otherwise.
    /// LET bindings are not yet supported with MATCH queries (v1.10) -- an explicit
    /// error is returned instead of silently discarding them.
    fn try_dispatch_match(
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
    fn compute_fetch_limit(stmt: &crate::velesql::SelectStatement) -> (usize, usize) {
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
    fn try_early_return_or_guard_let(
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
    fn prepare_query_context(
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
    #[allow(clippy::too_many_arguments)]
    fn finalize_query_results(
        &self,
        stmt: &crate::velesql::SelectStatement,
        results: &mut Vec<SearchResult>,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
        extracted: &ExtractedComponents,
        ctx: &crate::guardrails::QueryContext,
        let_bindings: &[crate::velesql::LetBinding],
    ) -> Result<()> {
        self.analyze_join_pushdown(stmt);
        self.check_guardrails_and_record(ctx, results.len())?;

        *results = self.apply_select_postprocessing(
            stmt,
            std::mem::take(results),
            params,
            limit,
            let_bindings,
        )?;

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
        Ok(())
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

    // NOTE: try_early_return_path, try_not_similarity_or_union, execute_early_return_query
    // moved to early_return.rs (NLOC/CC resolution batch 3)

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
