//! Sparse-only and hybrid dense+sparse query dispatch logic.
//!
//! Extracted from `mod.rs` to keep the main query orchestrator under 500 NLOC.
//! Contains the sparse query dispatch, hybrid search execution, graph-predicate
//! filtering, result finalization, and fusion strategy resolution.

use super::{distinct, Collection, ExtractedComponents, Result, SearchResult, MAX_LIMIT};
use tracing::warn;

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
        self.finalize_sparse_results(stmt, params, results)
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
        .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())
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
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())
        } else {
            self.execute_sparse_search(
                svs,
                params,
                extracted.filter_condition.as_ref(),
                execution_limit,
            )
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())
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
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure()),
            None => Ok(results),
        }
    }

    /// Applies DISTINCT, ORDER BY, OFFSET, and LIMIT to ranked results.
    ///
    /// Shared by the sparse/hybrid path and the NEAR_FUSED fusion path — both
    /// produce an already-ranked set that needs the same SQL-standard finalize.
    pub(super) fn finalize_sparse_results(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        mut results: Vec<SearchResult>,
    ) -> Result<Vec<SearchResult>> {
        if stmt.distinct == crate::velesql::DistinctMode::All {
            results = distinct::apply_distinct(results, &stmt.columns);
        }
        if let Some(ref order_by) = stmt.order_by {
            self.apply_order_by(&mut results, order_by, params)?;
        }
        // SQL-standard: OFFSET applied after ORDER BY, before LIMIT.
        // Intentional saturating clamp (also reached by the NEAR_FUSED path): an
        // out-of-`usize`-range offset saturates to `usize::MAX`, yielding an
        // empty page rather than an error.
        if let Some(offset) = stmt.offset {
            let skip = usize::try_from(offset).unwrap_or(usize::MAX);
            results = results.into_iter().skip(skip).collect();
        }
        // Intentional saturating clamp: a missing or out-of-range limit collapses
        // to `MAX_LIMIT` rather than erroring.
        let final_limit =
            usize::try_from(stmt.limit.unwrap_or(crate::velesql::DEFAULT_SELECT_LIMIT))
                .unwrap_or(MAX_LIMIT)
                .min(MAX_LIMIT);
        results.truncate(final_limit);
        self.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }

    /// Resolves the fusion strategy from the query's FUSION clause.
    pub(super) fn resolve_fusion_strategy(
        stmt: &crate::velesql::SelectStatement,
    ) -> crate::fusion::FusionStrategy {
        stmt.fusion_clause
            .as_ref()
            .map_or_else(crate::fusion::FusionStrategy::rrf_default, |fc| {
                use crate::velesql::FusionStrategyType;
                match fc.strategy {
                    FusionStrategyType::Rsf => {
                        let dw = fc.dense_weight.unwrap_or(0.5);
                        let sw = fc.sparse_weight.unwrap_or(0.5);
                        crate::fusion::FusionStrategy::relative_score(dw, sw)
                            .unwrap_or_else(|e| {
                                warn!(
                                    dense_weight = dw,
                                    sparse_weight = sw,
                                    error = %e,
                                    "RSF fusion strategy invalid; falling back to RRF"
                                );
                                crate::fusion::FusionStrategy::rrf_default()
                            })
                    }
                    FusionStrategyType::Rrf => crate::fusion::FusionStrategy::RRF {
                        k: fc.k.unwrap_or(60),
                    },
                    FusionStrategyType::Average => crate::fusion::FusionStrategy::Average,
                    FusionStrategyType::Maximum => crate::fusion::FusionStrategy::Maximum,
                    FusionStrategyType::Weighted => {
                        // 'weighted' = weighted Reciprocal Rank Fusion over the two
                        // branches (branch 0 = dense NEAR, branch 1 = sparse), honoring
                        // the dense_w/sparse_w from the FUSION clause. Falls back to RRF
                        // on a validation error (e.g. a negative weight).
                        let dw = fc.dense_weight.unwrap_or(0.5);
                        let sw = fc.sparse_weight.unwrap_or(0.5);
                        #[allow(clippy::cast_precision_loss)]
                        let k = fc.k.unwrap_or(60) as f32;
                        crate::fusion::FusionStrategy::weighted_rrf(vec![dw, sw], k)
                            .unwrap_or_else(|e| {
                                warn!(
                                    dense_weight = dw,
                                    sparse_weight = sw,
                                    k,
                                    error = %e,
                                    "Weighted RRF fusion strategy invalid; falling back to RRF"
                                );
                                crate::fusion::FusionStrategy::rrf_default()
                            })
                    }
                }
            })
    }
}
