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
pub(crate) mod set_operations;
mod similarity_filter;
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
    #[allow(clippy::too_many_lines)] // Complex dispatch logic - refactoring planned
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
            let match_results =
                self.execute_match_with_context(match_clause, params, Some(&ctx))?;

            // Check timeout after potentially expensive traversal.
            ctx.check_timeout()
                .map_err(crate::error::Error::from)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

            let mut sorted = match_results;
            if let Some(order_by) = match_clause.return_clause.order_by.as_ref() {
                for item in order_by.iter().rev() {
                    self.order_match_results(&mut sorted, &item.expression, item.descending);
                }
            }

            let mut results = self
                .match_results_to_search_results(sorted)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
            // Final cardinality check for MATCH path (EPIC-048 US-003).
            // execute_match_with_context only checks cardinality periodically every 100
            // traversal iterations. A query with <100 iterations that produces many results
            // (e.g., high fan-out from a single start node) bypasses the periodic check.
            // This explicit check on the final result set closes that gap.
            ctx.check_cardinality(results.len())
                .map_err(crate::error::Error::from)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
            if let Some(limit) = match_clause.return_clause.limit {
                let limit = usize::try_from(limit).unwrap_or(MAX_LIMIT).min(MAX_LIMIT);
                results.truncate(limit);
            }
            // Update QueryPlanner adaptive stats for graph/MATCH queries (Fix #8).
            // Reason: u128->u64 cast; query durations < u64::MAX µs (~585 millennia)
            #[allow(clippy::cast_possible_truncation)]
            let graph_latency_us = ctx.elapsed().as_micros() as u64;
            self.query_planner
                .stats()
                .update_graph_latency(graph_latency_us);
            self.guard_rails.circuit_breaker.record_success();
            return Ok(results);
        }

        let stmt = &query.select;
        // Cap limit to prevent overflow in over-fetch calculations
        let limit = usize::try_from(stmt.limit.unwrap_or(10))
            .unwrap_or(MAX_LIMIT)
            .min(MAX_LIMIT);

        // 1. Extract vector search (NEAR) or similarity() conditions if present
        let mut vector_search = None;
        let mut similarity_conditions: Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)> =
            Vec::new();
        let mut filter_condition = None;
        let mut graph_match_predicates = Vec::new();

        // EPIC-044 US-002: Check for similarity() OR metadata pattern (union mode)
        let is_union_query = if let Some(ref cond) = stmt.where_clause {
            Self::has_similarity_in_problematic_or(cond)
        } else {
            false
        };

        // EPIC-044 US-003: Check for NOT similarity() pattern (scan mode)
        let is_not_similarity_query = if let Some(ref cond) = stmt.where_clause {
            Self::has_similarity_under_not(cond)
        } else {
            false
        };

        // Extract sparse vector search condition (Phase 5 hybrid support).
        let mut sparse_vector_search = None;

        if let Some(ref cond) = stmt.where_clause {
            // Validate query structure before extraction
            Self::validate_similarity_query_structure(cond)?;
            Self::collect_graph_match_predicates(cond, &mut graph_match_predicates);

            // Check for SPARSE_NEAR before cloning for dense extraction.
            sparse_vector_search = Self::extract_sparse_vector_search(cond).cloned();

            // Reason: extract_vector_search mutates the condition in-place to remove the NEAR node;
            // the original cond is still needed for similarity/filter extraction below.
            // Clone is unavoidable until extract_vector_search returns a new condition instead.
            let mut extracted_cond = cond.clone();
            vector_search = self.extract_vector_search(&mut extracted_cond, params)?;
            // EPIC-044 US-001: Extract ALL similarity conditions for cascade filtering
            similarity_conditions =
                self.extract_all_similarity_conditions(&extracted_cond, params)?;
            filter_condition = Some(extracted_cond);

            // NEAR + similarity() is supported: NEAR finds candidates, similarity() filters by threshold
            // Multiple similarity() with AND is supported: filters applied sequentially (cascade)
        }

        // 2. Resolve WITH clause options
        let mut ef_search = None;
        if let Some(ref with) = stmt.with_clause {
            ef_search = with.get_ef_search();
        }

        // Get first similarity condition for initial search (if any)
        let first_similarity = similarity_conditions.first().cloned();
        let has_graph_predicates = !graph_match_predicates.is_empty();
        let skip_metadata_prefilter_for_graph_or = has_graph_predicates
            && stmt
                .where_clause
                .as_ref()
                .is_some_and(Self::condition_contains_or);
        let execution_limit = if has_graph_predicates {
            MAX_LIMIT
        } else {
            limit
        };

        // 3a. CBO: Choose execution strategy via QueryPlanner (EPIC-046).
        //
        // Uses choose_strategy_with_cbo_and_overfetch() which returns both the strategy
        // and the selectivity-derived over-fetch factor in a single pass.
        let (cbo_strategy, cbo_over_fetch) = {
            let col_stats = self.get_stats();
            self.query_planner.choose_strategy_with_cbo_and_overfetch(
                &col_stats,
                filter_condition.as_ref(),
                limit,
            )
        };
        tracing::debug!(
            strategy = ?cbo_strategy,
            over_fetch = cbo_over_fetch,
            "CBO selected execution strategy"
        );

        // 3. Execute query based on extracted components
        // EPIC-044 US-003: NOT similarity() requires full scan
        if is_not_similarity_query {
            if let Some(ref cond) = stmt.where_clause {
                let mut results = self
                    .execute_not_similarity_query(cond, params, execution_limit)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                if has_graph_predicates {
                    results = self
                        .apply_where_condition_to_results(results, cond, params, &stmt.from_alias)
                        .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                }

                // Apply ORDER BY if present
                if let Some(ref order_by) = stmt.order_by {
                    self.apply_order_by(&mut results, order_by, params)
                        .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                }
                results.truncate(limit);
                // Guard-rail checks for early-return paths (EPIC-048 US-001, US-003).
                // These paths return before the main-path checks at the bottom of execute_query.
                // NOT similarity() is a full table scan — timeout and cardinality MUST be enforced.
                ctx.check_timeout()
                    .map_err(crate::error::Error::from)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                ctx.check_cardinality(results.len())
                    .map_err(crate::error::Error::from)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                self.guard_rails.circuit_breaker.record_success();
                return Ok(results);
            }
        }

        // EPIC-044 US-002: Union mode for similarity() OR metadata
        if is_union_query {
            if let Some(ref cond) = stmt.where_clause {
                let mut results = self
                    .execute_union_query(cond, params, execution_limit)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                if has_graph_predicates {
                    results = self
                        .apply_where_condition_to_results(results, cond, params, &stmt.from_alias)
                        .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                }

                // Apply ORDER BY if present
                if let Some(ref order_by) = stmt.order_by {
                    self.apply_order_by(&mut results, order_by, params)
                        .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                }
                results.truncate(limit);
                // Guard-rail checks for early-return paths (EPIC-048 US-001, US-003).
                // Union queries return here without reaching the main-path guard-rail checks.
                ctx.check_timeout()
                    .map_err(crate::error::Error::from)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                ctx.check_cardinality(results.len())
                    .map_err(crate::error::Error::from)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                self.guard_rails.circuit_breaker.record_success();
                return Ok(results);
            }
        }

        // Phase 5: Sparse-only or hybrid dense+sparse execution.
        if let Some(ref svs) = sparse_vector_search {
            let mut results = if let Some(ref dense_vec) = vector_search {
                // Hybrid: dense NEAR + SPARSE_NEAR -> parallel execution + fusion.
                let fusion_strategy = stmt.fusion_clause.as_ref().map_or_else(
                    crate::fusion::FusionStrategy::rrf_default,
                    |fc| {
                        use crate::velesql::FusionStrategyType;
                        match fc.strategy {
                            FusionStrategyType::Rsf => {
                                let dw = fc.dense_weight.unwrap_or(0.5);
                                let sw = fc.sparse_weight.unwrap_or(0.5);
                                crate::fusion::FusionStrategy::relative_score(dw, sw)
                                    .unwrap_or_else(|_| {
                                        crate::fusion::FusionStrategy::rrf_default()
                                    })
                            }
                            FusionStrategyType::Rrf => crate::fusion::FusionStrategy::RRF {
                                k: fc.k.unwrap_or(60),
                            },
                            _ => crate::fusion::FusionStrategy::rrf_default(),
                        }
                    },
                );
                self.execute_hybrid_search_with_strategy(
                    dense_vec,
                    svs,
                    params,
                    filter_condition.as_ref(),
                    execution_limit,
                    &fusion_strategy,
                )
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?
            } else {
                // Sparse-only: no dense NEAR.
                self.execute_sparse_search(svs, params, filter_condition.as_ref(), execution_limit)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?
            };

            if has_graph_predicates {
                if let Some(cond) = stmt.where_clause.as_ref() {
                    results = self
                        .apply_where_condition_to_results(results, cond, params, &stmt.from_alias)
                        .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
                }
            }

            ctx.check_timeout()
                .map_err(crate::error::Error::from)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
            ctx.check_cardinality(results.len())
                .map_err(crate::error::Error::from)
                .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

            if stmt.distinct == crate::velesql::DistinctMode::All {
                results = distinct::apply_distinct(results, &stmt.columns);
            }
            if let Some(ref order_by) = stmt.order_by {
                self.apply_order_by(&mut results, order_by, params)?;
            }
            results.truncate(limit);
            self.guard_rails.circuit_breaker.record_success();
            return Ok(results);
        }

        // EPIC-044 US-001: Support multiple similarity() with AND (cascade filtering).
        // Dispatch delegated to dispatch_vector_query() in execution_paths.rs.
        let mut results = self
            .dispatch_vector_query(
                vector_search.as_ref(),
                first_similarity.as_ref(),
                &similarity_conditions,
                filter_condition.as_ref(),
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
                ef_search,
                cbo_strategy,
                cbo_over_fetch,
            )
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

        if has_graph_predicates {
            if let Some(cond) = stmt.where_clause.as_ref() {
                results = self
                    .apply_where_condition_to_results(results, cond, params, &stmt.from_alias)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
            }
        }

        // P1-B: Filter pushdown analysis for JOIN queries (EPIC-031 US-006).
        //
        // When the query has JOIN clauses, analyze WHERE conditions to classify filters:
        // - Graph/payload filters → already applied above
        // - ColumnStore filters → logged for future cross-store JOIN execution
        // - Post-JOIN filters → deferred (require cross-store data)
        //
        // This wires up `analyze_for_pushdown` to activate the classification,
        // enabling the future ColumnStore JOIN executor to consume the analysis.
        if !stmt.joins.is_empty() {
            if let Some(ref cond) = stmt.where_clause {
                // BUG-8 fix: from_alias is now Vec<String> containing all aliases
                // visible in scope (FROM alias + JOIN aliases).
                let graph_vars: std::collections::HashSet<String> =
                    stmt.from_alias.iter().cloned().collect();
                let join_tables = pushdown::extract_join_tables(&stmt.joins);
                let analysis = pushdown::analyze_for_pushdown(cond, &graph_vars, &join_tables);
                tracing::debug!(
                    column_store_filters = analysis.column_store_filters.len(),
                    graph_filters = analysis.graph_filters.len(),
                    post_join_filters = analysis.post_join_filters.len(),
                    has_pushdown = analysis.has_pushdown(),
                    "JOIN pushdown analysis complete"
                );
            }
        }

        // Check timeout after all search/filter operations (EPIC-048 US-001).
        ctx.check_timeout()
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

        // Check cardinality on final result set (EPIC-048 US-003).
        // check_cardinality uses fetch_add internally; since this is the only call on this
        // ctx for the SELECT path, passing results.len() as the delta is correct.
        ctx.check_cardinality(results.len())
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

        // EPIC-052 US-001: Apply DISTINCT deduplication if requested
        if stmt.distinct == crate::velesql::DistinctMode::All {
            results = distinct::apply_distinct(results, &stmt.columns);
        }

        // Apply ORDER BY if present
        if let Some(ref order_by) = stmt.order_by {
            self.apply_order_by(&mut results, order_by, params)?;
        }

        // Apply limit
        results.truncate(limit);

        // Update QueryPlanner adaptive stats for vector/SELECT queries (Fix #8).
        // Only record vector latency when a NEAR search was performed.
        if vector_search.is_some() {
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
