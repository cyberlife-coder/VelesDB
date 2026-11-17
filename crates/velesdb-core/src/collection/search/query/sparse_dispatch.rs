//! Sparse-only and hybrid dense+sparse query dispatch logic.
//!
//! Extracted from `mod.rs` to keep the main query orchestrator under 500 NLOC.
//! Contains the sparse query dispatch, hybrid search execution, graph-predicate
//! filtering, result finalization, and fusion strategy resolution.

use super::{Collection, ExtractedComponents, Result, SearchResult, MAX_LIMIT};

impl Collection {
    /// Dispatches sparse-only or hybrid dense+sparse search.
    pub(super) fn dispatch_sparse_query(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        svs: &crate::velesql::SparseVectorSearch,
        limit: usize,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let has_graph_predicates = !extracted.graph_match_predicates.is_empty();

        // GraphFirst by anchor ids (sparse-only): AND-required MATCH
        // predicates restrict the sparse fetch via the index's per-id
        // filter, so retrieval is exact at `limit` within the graph matches
        // instead of post-filtering a MAX_LIMIT window. Hybrid dense+sparse
        // keeps the window: its fusion legs rank independently.
        let mut graph_cache = super::where_eval::GraphMatchEvalCache::default();
        let anchors = self.sparse_anchor_prefilter(stmt, params, extracted, &mut graph_cache)?;

        let mut results =
            self.fetch_sparse_results(stmt, params, extracted, svs, limit, anchors.as_ref())?;

        if has_graph_predicates {
            results = self.filter_by_graph_predicates_with_cache(
                stmt,
                params,
                results,
                &mut graph_cache,
            )?;
        }

        self.check_guardrails_and_record(ctx, results.len())?;
        let results = self.finalize_ranked_results(stmt, params, results)?;
        self.runtime.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }

    /// Fetches sparse/hybrid results: anchored exact fetch when a GraphFirst
    /// anchor set is available, MAX_LIMIT window fetch otherwise.
    fn fetch_sparse_results(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        svs: &crate::velesql::SparseVectorSearch,
        limit: usize,
        anchors: Option<&std::collections::HashSet<u64>>,
    ) -> Result<Vec<SearchResult>> {
        let Some(anchor_ids) = anchors else {
            let execution_limit = if extracted.graph_match_predicates.is_empty() {
                limit
            } else {
                MAX_LIMIT
            };
            return self.execute_sparse_or_hybrid(stmt, extracted, svs, params, execution_limit);
        };
        self.execute_sparse_search_in_anchors(
            svs,
            params,
            extracted.filter_condition.as_ref(),
            limit,
            Some(anchor_ids),
        )
        .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))
    }

    /// Computes the GraphFirst anchor set for a sparse-only fetch.
    ///
    /// Returns `None` for hybrid dense+sparse queries and for residual
    /// conditions the in-fetch filter cannot cover (text MATCH,
    /// similarity()) — those drop rows after the fetch and keep the window.
    fn sparse_anchor_prefilter(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Option<std::collections::HashSet<u64>>> {
        if extracted.graph_match_predicates.is_empty()
            || extracted.vector_search.is_some()
            || !extracted.similarity_conditions.is_empty()
        {
            return Ok(None);
        }
        let Some(cond) = stmt.where_clause.as_ref() else {
            return Ok(None);
        };
        if Self::extract_match_query(cond).is_some() {
            return Ok(None);
        }
        self.compute_required_anchor_ids(cond, params, &stmt.from_alias, graph_cache)
    }

    /// Executes either a sparse-only or hybrid dense+sparse search.
    fn execute_sparse_or_hybrid(
        &self,
        stmt: &crate::velesql::SelectStatement,
        extracted: &ExtractedComponents,
        svs: &crate::velesql::SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        execution_limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if let Some(ref dense_vec) = extracted.vector_search {
            let fusion_strategy = Self::resolve_fusion_strategy(stmt);
            self.execute_hybrid_search_with_strategy(
                dense_vec,
                svs,
                params,
                extracted.filter_condition.as_ref(),
                execution_limit,
                &fusion_strategy,
            )
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))
        } else {
            self.execute_sparse_search(
                svs,
                params,
                extracted.filter_condition.as_ref(),
                execution_limit,
            )
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))
        }
    }

    /// Applies graph-predicate WHERE filtering to results, reusing the
    /// caller's evaluation cache so prefiltered anchor sets are not
    /// re-evaluated.
    fn filter_by_graph_predicates_with_cache(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        results: Vec<SearchResult>,
        cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Vec<SearchResult>> {
        match stmt.where_clause.as_ref() {
            Some(cond) => self
                .apply_where_condition_to_results_with_cache(
                    results,
                    cond,
                    params,
                    &stmt.from_alias,
                    cache,
                )
                .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e)),
            None => Ok(results),
        }
    }

    /// The SELECT post-processing of a path that ranks its own rows (the
    /// sparse/hybrid search, the `NEAR_FUSED` fusion, the exact
    /// `similarity()` scan): a vector `GROUP BY` groups the ranked rows, as
    /// the main pipeline's `dispatch_and_finalize` does, then the main
    /// pipeline's post-processing ([`Self::apply_select_postprocessing`]:
    /// DISTINCT, window functions, ORDER BY, OFFSET) runs at the statement's
    /// own `LIMIT`. LET bindings never reach these paths
    /// (`validate_let_binding_support`).
    pub(super) fn finalize_ranked_results(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        results: Vec<SearchResult>,
    ) -> Result<Vec<SearchResult>> {
        if !stmt.is_vector_group_by() {
            return self.postprocess_ranked_results(stmt, params, results);
        }
        let groups = self.apply_vector_group_by(stmt, &results, params)?;
        let stmt = stmt
            .order_by_aggregates_as_fields()
            .map_err(|e| crate::error::Error::Query(e.to_string()))?;
        self.postprocess_ranked_results(&stmt, params, groups)
    }

    /// [`Self::apply_select_postprocessing`] at the statement's own `LIMIT`.
    fn postprocess_ranked_results(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        results: Vec<SearchResult>,
    ) -> Result<Vec<SearchResult>> {
        // Intentional saturating clamp: a missing or out-of-range limit collapses
        // to `MAX_LIMIT` rather than erroring.
        let final_limit =
            usize::try_from(stmt.limit.unwrap_or(crate::velesql::DEFAULT_SELECT_LIMIT))
                .unwrap_or(MAX_LIMIT)
                .min(MAX_LIMIT);
        self.apply_select_postprocessing(stmt, results, params, final_limit, &[])
    }

    /// Resolves the fusion strategy from the query's FUSION clause: RRF when
    /// there is none ([`crate::velesql::FusionClause::dense_sparse_strategy`]).
    pub(super) fn resolve_fusion_strategy(
        stmt: &crate::velesql::SelectStatement,
    ) -> crate::fusion::FusionStrategy {
        stmt.fusion_clause.as_ref().map_or_else(
            crate::fusion::FusionStrategy::rrf_default,
            crate::velesql::FusionClause::dense_sparse_strategy,
        )
    }
}
