//! Early-return query paths: the exact similarity() scan (negated or
//! OR-ed thresholds), sparse and fused dispatch.
//!
//! Extracted from `query/mod.rs` to reduce NLOC below the 500 threshold.

use super::options::MAX_LIMIT;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

/// Context for the early-return similarity() scan.
pub(super) struct EarlyReturnCtx<'a> {
    pub(super) stmt: &'a crate::velesql::SelectStatement,
    pub(super) params: &'a std::collections::HashMap<String, serde_json::Value>,
    pub(super) cond: &'a crate::velesql::Condition,
    pub(super) has_graph_predicates: bool,
    pub(super) ctx: &'a crate::guardrails::QueryContext,
}

impl Collection {
    /// Attempts early-return paths: the similarity() scan, sparse and fused
    /// queries.
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

        // A scalar ORDER BY ranks rows only after the fetch, so the sparse and
        // fused fetches below must not stop at `limit` first — the reason
        // `try_not_similarity_or_union` widens its scan.
        let limit = if Self::order_by_requires_exhaustive_fetch(stmt) {
            MAX_LIMIT
        } else {
            limit
        };

        // Phase 5: Sparse-only or hybrid dense+sparse execution.
        if let Some(ref svs) = extracted.sparse_vector_search {
            let results = self.dispatch_sparse_query(stmt, params, extracted, svs, limit, ctx)?;
            return Ok(Some(results));
        }

        // NEAR_FUSED multi-vector fusion, routed to multi_query_search.
        if extracted.fused_search.is_some() {
            let results = self.dispatch_fused_query(stmt, params, extracted, limit, ctx)?;
            return Ok(Some(results));
        }

        Ok(None)
    }

    /// Handles the shapes no index can bound — a negated `similarity()`, or
    /// one under `OR` — by the exact scan.
    fn try_not_similarity_or_union(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &super::ExtractedComponents,
        limit: usize,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Option<Vec<SearchResult>>> {
        let cond = match stmt.where_clause.as_ref() {
            Some(c) if extracted.similarity_scan => c,
            _ => return Ok(None),
        };

        let has_graph_predicates = !extracted.graph_match_predicates.is_empty();
        let early_ctx = EarlyReturnCtx {
            stmt,
            params,
            cond,
            has_graph_predicates,
            ctx,
        };

        // A scalar (non-similarity) ORDER BY ranks rows only in
        // `finalize_early_results`, so capping the fetch at `limit` first would
        // truncate before the sort (same defect as the main-dispatch path).
        // Fetch exhaustively so the sort precedes truncation.
        let limit = if Self::order_by_requires_exhaustive_fetch(stmt) {
            MAX_LIMIT
        } else {
            limit
        };

        self.run_similarity_scan_early(&early_ctx, limit).map(Some)
    }

    /// Runs the exact `similarity()` scan (`similarity_scan.rs`) with
    /// GraphFirst anchoring: AND-required MATCH predicates restrict the scan
    /// to their anchor set, making the fetch exact at `limit` instead of a
    /// MAX_LIMIT window.
    fn run_similarity_scan_early(
        &self,
        early: &EarlyReturnCtx<'_>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let mut graph_cache = super::where_eval::GraphMatchEvalCache::default();
        let anchors = if early.has_graph_predicates {
            self.compute_required_anchor_ids(
                early.cond,
                early.params,
                &early.stmt.from_alias,
                &mut graph_cache,
            )?
        } else {
            None
        };
        let execution_limit = if early.has_graph_predicates && anchors.is_none() {
            MAX_LIMIT
        } else {
            limit
        };
        self.execute_early_return_query(
            |s| {
                s.execute_similarity_scan(
                    early.cond,
                    early.params,
                    execution_limit,
                    anchors.as_ref(),
                )
            },
            early,
            &mut graph_cache,
        )
    }

    /// Executes an early-return query path with guard-rail checks and post-processing.
    ///
    /// `graph_cache` carries anchor sets a GraphFirst prefilter already
    /// computed, so the exact post-filter never re-runs the traversals.
    pub(super) fn execute_early_return_query(
        &self,
        execute_fn: impl FnOnce(&Self) -> Result<Vec<SearchResult>>,
        early: &EarlyReturnCtx<'_>,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Vec<SearchResult>> {
        let mut results = execute_fn(self)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        if early.has_graph_predicates {
            results = self
                .apply_where_condition_to_results_with_cache(
                    results,
                    early.cond,
                    early.params,
                    &early.stmt.from_alias,
                    graph_cache,
                )
                .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        }
        self.finalize_early_results(results, early)
    }

    /// The SELECT post-processing of the scan's rows
    /// ([`Self::finalize_ranked_results`]), then the guard-rails.
    fn finalize_early_results(
        &self,
        results: Vec<SearchResult>,
        early: &EarlyReturnCtx<'_>,
    ) -> Result<Vec<SearchResult>> {
        let results = self
            .finalize_ranked_results(early.stmt, early.params, results)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        self.check_guardrails_and_record(early.ctx, results.len())?;
        self.runtime.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }
}
