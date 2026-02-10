//! VelesQL query execution for Collection.
//!
//! This module orchestrates query execution by combining:
//! - Query validation (`validation.rs`)
//! - Condition extraction (`extraction.rs`)
//! - ORDER BY processing (`ordering.rs`)
//!
//! # Cross-Store Execution (VP-010, Phase 7)
//!
//! Combined NEAR + graph MATCH queries are dispatched via `QueryPlanner::choose_hybrid_strategy()`
//! which selects VectorFirst, Parallel, or GraphFirst execution based on:
//! - Query pattern (ORDER BY similarity, filters, etc.)
//! - Runtime statistics (latency, selectivity)
//! - Over-fetch factor for filtered queries
//!
//! See `cross_store_exec.rs` for VectorFirst and Parallel implementations.

// SAFETY: Numeric casts in query execution are intentional:
// - f64->f32 for similarity thresholds: precision loss acceptable for filtering
// - Thresholds are approximate bounds, exact precision not required
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]

mod aggregation;
#[cfg(test)]
mod bm25_integration_tests;
pub mod compound;
#[cfg(test)]
mod compound_tests;
mod cross_store_exec;
#[cfg(test)]
mod cross_store_exec_tests;
#[cfg(test)]
mod cross_store_tests;
mod dispatch;
mod distinct;
mod extraction;
#[cfg(test)]
mod extraction_tests;
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
#[cfg(test)]
mod match_return_agg_tests;
#[cfg(test)]
mod match_where_eval_tests;
#[cfg(test)]
mod near_fused_tests;
mod ordering;
pub mod parallel_traversal;
#[cfg(test)]
mod parallel_traversal_tests;
pub mod pushdown;
#[cfg(test)]
mod pushdown_tests;
pub mod score_fusion;
#[cfg(test)]
mod score_fusion_tests;
mod similarity_filter;
mod subquery;
#[cfg(test)]
mod subquery_tests;
mod union_query;
mod validation;

// Re-export for potential external use
#[allow(unused_imports)]
pub use ordering::compare_json_values;
// Re-export join functions for future integration with execute_query
#[allow(unused_imports)]
pub use join::{execute_join, JoinedResult};

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

/// Maximum allowed LIMIT value to prevent overflow in over-fetch calculations.
const MAX_LIMIT: usize = 100_000;

impl Collection {
    /// Executes a `VelesQL` query on this collection.
    ///
    /// This method unifies vector search, text search, and metadata filtering
    /// into a single interface. Dispatch logic is in `dispatch.rs`.
    ///
    /// # Arguments
    ///
    /// * `query` - Parsed `VelesQL` query
    /// * `params` - Query parameters for resolving placeholders (e.g., $v)
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed (e.g., missing parameters).
    #[allow(clippy::too_many_lines)]
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let stmt = &query.select;
        let limit = usize::try_from(stmt.limit.unwrap_or(10))
            .unwrap_or(MAX_LIMIT)
            .min(MAX_LIMIT);

        // 1. Extract conditions from WHERE clause
        let mut vector_search = None;
        let mut similarity_conditions: Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)> =
            Vec::new();
        let mut filter_condition = None;

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
            let mut extracted_cond = cond.clone();
            vector_search = self.extract_vector_search(&mut extracted_cond, params)?;
            similarity_conditions =
                self.extract_all_similarity_conditions(&extracted_cond, params)?;
            filter_condition = Some(extracted_cond);

            // VP-002: Resolve subquery values before filter conversion
            if let Some(ref cond) = filter_condition {
                filter_condition = Some(self.resolve_subqueries_in_condition(cond, params)?);
            }
        }

        // 2. Resolve WITH clause options
        let mut ef_search = None;
        let mut overfetch_base: usize = 10;
        if let Some(ref with) = stmt.with_clause {
            ef_search = with.get_ef_search();
            if let Some(of) = with.get_overfetch() {
                overfetch_base = of;
            }
        }

        let first_similarity = similarity_conditions.first().cloned();

        // 3. Early-return dispatch paths
        // EPIC-044 US-003: NOT similarity() requires full scan
        if is_not_similarity_query {
            if let Some(ref cond) = stmt.where_clause {
                let mut results = self.execute_not_similarity_query(cond, params, limit)?;
                if let Some(ref order_by) = stmt.order_by {
                    self.apply_order_by(&mut results, order_by, params)?;
                }
                results.truncate(limit);
                return Ok(results);
            }
        }

        // EPIC-044 US-002: Union mode for similarity() OR metadata
        if is_union_query {
            if let Some(ref cond) = stmt.where_clause {
                let mut results = self.execute_union_query(cond, params, limit, overfetch_base)?;
                if let Some(ref order_by) = stmt.order_by {
                    self.apply_order_by(&mut results, order_by, params)?;
                }
                results.truncate(limit);
                return Ok(results);
            }
        }

        // VP-012: NEAR_FUSED dispatch
        if let Some(ref cond) = stmt.where_clause {
            if let Some(results) = self.dispatch_fused_search(
                cond,
                params,
                vector_search.as_deref(),
                &similarity_conditions,
                limit,
                stmt.order_by.as_deref(),
            )? {
                return Ok(results);
            }
        }

        // VP-010: Cross-store dispatch (NEAR + graph MATCH)
        if let Some(ref match_clause) = query.match_clause {
            if let Some(ref vector) = vector_search {
                let planner = crate::velesql::QueryPlanner::new();
                let has_order_by_sim = stmt.order_by.as_ref().is_some_and(|obs| {
                    obs.iter()
                        .any(|o| matches!(o.expr, crate::velesql::OrderByExpr::Similarity(_)))
                });
                let has_filter = filter_condition
                    .as_ref()
                    .is_some_and(|c| Self::extract_metadata_filter(c).is_some());
                let plan =
                    planner.choose_hybrid_strategy(has_order_by_sim, has_filter, stmt.limit, None);
                let mut results = match plan.strategy {
                    crate::velesql::ExecutionStrategy::VectorFirst => {
                        self.execute_vector_first_cross_store(vector, match_clause, params, limit)?
                    }
                    crate::velesql::ExecutionStrategy::Parallel => {
                        self.execute_parallel_cross_store(vector, match_clause, params, limit)?
                    }
                    crate::velesql::ExecutionStrategy::GraphFirst => {
                        let match_results =
                            self.execute_match_with_similarity(match_clause, vector, 0.0, params)?;
                        self.match_results_to_search_results(match_results)?
                    }
                };
                if let Some(ref order_by) = stmt.order_by {
                    self.apply_order_by(&mut results, order_by, params)?;
                }
                results.truncate(limit);
                return Ok(results);
            }
        }

        // 4. Main dispatch (delegated to dispatch.rs)
        let ctx = dispatch::QueryContext {
            vector_search: vector_search.as_deref(),
            first_similarity,
            all_similarity_conditions: &similarity_conditions,
            filter_condition: filter_condition.as_ref(),
            limit,
            ef_search,
            overfetch_base,
        };
        let mut results = self.dispatch_main_query(&ctx)?;

        // 5. Post-processing
        if stmt.distinct == crate::velesql::DistinctMode::All {
            results = distinct::apply_distinct(results, &stmt.columns);
        }
        if let Some(ref order_by) = stmt.order_by {
            self.apply_order_by(&mut results, order_by, params)?;
        }
        results.truncate(limit);

        Ok(results)
    }

    // NOTE: apply_distinct and compute_distinct_key moved to distinct.rs
    // (EPIC-061/US-003 refactoring)

    // NOTE: filter_by_similarity, execute_not_similarity_query, extract_not_similarity_condition,
    // execute_scan_query moved to similarity_filter.rs (Plan 04-04)

    // NOTE: execute_union_query, matches_metadata_filter, split_or_condition_with_outer_filter
    // moved to union_query.rs (Plan 04-04)
}
