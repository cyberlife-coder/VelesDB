//! Early-return query paths: NOT-similarity, union, and sparse dispatch.
//!
//! Extracted from `query/mod.rs` to reduce NLOC below the 500 threshold.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

/// Maximum allowed LIMIT value to prevent overflow in over-fetch calculations.
const MAX_LIMIT: usize = 100_000;

/// Context for early-return query paths (NOT-similarity, union).
pub(super) struct EarlyReturnCtx<'a> {
    pub(super) stmt: &'a crate::velesql::SelectStatement,
    pub(super) params: &'a std::collections::HashMap<String, serde_json::Value>,
    pub(super) cond: &'a crate::velesql::Condition,
    pub(super) has_graph_predicates: bool,
    pub(super) ctx: &'a crate::guardrails::QueryContext,
}

impl Collection {
    /// Attempts early-return paths: NOT-similarity, union, and sparse queries.
    ///
    /// Returns `Ok(Some(results))` if an early path was taken, `Ok(None)` otherwise.
    pub(super) fn try_early_return_path(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &super::ExtractedComponents,
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
        extracted: &super::ExtractedComponents,
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
    pub(super) fn execute_early_return_query(
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
        // Bug #475: Apply DISTINCT before ORDER BY (same as finalize_query_results path).
        if early.stmt.distinct == crate::velesql::DistinctMode::All {
            results = super::distinct::apply_distinct(results, &early.stmt.columns);
        }
        if let Some(ref order_by) = early.stmt.order_by {
            self.apply_order_by(&mut results, order_by, early.params)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        }
        // SQL-standard: OFFSET applied after ORDER BY, before LIMIT.
        if let Some(offset) = early.stmt.offset {
            let skip = usize::try_from(offset).unwrap_or(usize::MAX);
            results = results.into_iter().skip(skip).collect();
        }
        let final_limit = usize::try_from(early.stmt.limit.unwrap_or(10))
            .unwrap_or(MAX_LIMIT)
            .min(MAX_LIMIT);
        results.truncate(final_limit);
        self.check_guardrails_and_record(early.ctx, results.len())?;
        self.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }
}
