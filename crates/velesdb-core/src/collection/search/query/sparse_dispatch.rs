//! Sparse-only and hybrid dense+sparse query dispatch logic.
//!
//! Extracted from `mod.rs` to keep the main query orchestrator under 500 NLOC.
//! Contains the sparse query dispatch, hybrid search execution, graph-predicate
//! filtering, result finalization, and fusion strategy resolution.

use super::{distinct, Collection, ExtractedComponents, Result, SearchResult, MAX_LIMIT};

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
        let execution_limit = if has_graph_predicates {
            MAX_LIMIT
        } else {
            limit
        };

        let mut results =
            self.execute_sparse_or_hybrid(stmt, extracted, svs, params, execution_limit)?;

        if has_graph_predicates {
            results = self.filter_by_graph_predicates(stmt, params, results)?;
        }

        self.check_guardrails_and_record(ctx, results.len())?;
        self.finalize_sparse_results(stmt, params, results)
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

    /// Applies graph-predicate WHERE filtering to results.
    fn filter_by_graph_predicates(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        results: Vec<SearchResult>,
    ) -> Result<Vec<SearchResult>> {
        match stmt.where_clause.as_ref() {
            Some(cond) => self
                .apply_where_condition_to_results(results, cond, params, &stmt.from_alias)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure()),
            None => Ok(results),
        }
    }

    /// Applies DISTINCT, ORDER BY, and LIMIT to sparse/hybrid results.
    fn finalize_sparse_results(
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
        if let Some(offset) = stmt.offset {
            let skip = usize::try_from(offset).unwrap_or(usize::MAX);
            results = results.into_iter().skip(skip).collect();
        }
        let final_limit = usize::try_from(stmt.limit.unwrap_or(10))
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
                            .unwrap_or_else(|_| crate::fusion::FusionStrategy::rrf_default())
                    }
                    FusionStrategyType::Rrf => crate::fusion::FusionStrategy::RRF {
                        k: fc.k.unwrap_or(60),
                    },
                    FusionStrategyType::Average => crate::fusion::FusionStrategy::Average,
                    FusionStrategyType::Maximum => crate::fusion::FusionStrategy::Maximum,
                    FusionStrategyType::Weighted => crate::fusion::FusionStrategy::rrf_default(),
                }
            })
    }
}
