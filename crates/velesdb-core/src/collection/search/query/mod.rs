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

// SAFETY: Numeric casts in query execution are intentional:
// - f64->f32 for similarity thresholds: precision loss acceptable for filtering
// - Thresholds are approximate bounds, exact precision not required
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::uninlined_format_args)] // Prefer readability in query error paths.
#![allow(clippy::implicit_hasher)] // HashSet hasher genericity adds noise for internal APIs.

mod aggregation;
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
    /// into a single interface.
    ///
    /// # Arguments
    ///
    /// * `query` - Parsed `VelesQL` query
    /// * `params` - Query parameters for resolving placeholders (e.g., $v)
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed (e.g., missing parameters).
    #[allow(clippy::too_many_lines)] // Complex dispatch logic - refactoring planned
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
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

        if let Some(ref cond) = stmt.where_clause {
            // Validate query structure before extraction
            Self::validate_similarity_query_structure(cond)?;

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

        // 3. Execute query based on extracted components
        // EPIC-044 US-003: NOT similarity() requires full scan
        if is_not_similarity_query {
            if let Some(ref cond) = stmt.where_clause {
                let mut results = self.execute_not_similarity_query(cond, params, limit)?;

                // Apply ORDER BY if present
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
                let mut results = self.execute_union_query(cond, params, limit)?;

                // Apply ORDER BY if present
                if let Some(ref order_by) = stmt.order_by {
                    self.apply_order_by(&mut results, order_by, params)?;
                }
                results.truncate(limit);
                return Ok(results);
            }
        }

        // EPIC-044 US-001: Support multiple similarity() with AND (cascade filtering)
        let mut results = match (&vector_search, &first_similarity, &filter_condition) {
            // similarity() function - use first vector to search, then filter by ALL thresholds
            // Also apply any additional metadata filters from the WHERE clause
            //
            // NOTE: This uses ANN (top-K) search, not exhaustive search.
            // Points outside the top-K window may match the threshold but won't be returned.
            // We use a 10x over-fetch factor to reduce false negatives.
            (None, Some((field, vec, op, threshold)), filter_cond) => {
                // Validate field name - currently only "vector" is supported
                if field != "vector" {
                    return Err(crate::error::Error::Config(format!(
                        "similarity() field '{}' not found. Only 'vector' field is supported. \
                        Multi-vector support is planned for a future release.",
                        field
                    )));
                }

                // Increase over-fetch factor for multiple similarity conditions
                let overfetch_factor = 10 * similarity_conditions.len().max(1);
                let candidates_k = limit.saturating_mul(overfetch_factor).min(MAX_LIMIT);
                let candidates = self.search(vec, candidates_k)?;

                // EPIC-044 US-001: Apply ALL similarity filters sequentially (cascade)
                let filter_k = limit.saturating_mul(2);
                let mut filtered =
                    self.filter_by_similarity(candidates, field, vec, *op, *threshold, filter_k);

                // Apply remaining similarity conditions (cascade filtering)
                for (sim_field, sim_vec, sim_op, sim_threshold) in
                    similarity_conditions.iter().skip(1)
                {
                    if sim_field != "vector" {
                        return Err(crate::error::Error::Config(format!(
                            "similarity() field '{}' not found. Only 'vector' field is supported.",
                            sim_field
                        )));
                    }
                    filtered = self.filter_by_similarity(
                        filtered,
                        sim_field,
                        sim_vec,
                        *sim_op,
                        *sim_threshold,
                        filter_k,
                    );
                }

                // Then apply any additional metadata filters (e.g., AND category = 'tech')
                if let Some(cond) = filter_cond {
                    let metadata_filter = Self::extract_metadata_filter(cond);
                    if let Some(filter_cond) = metadata_filter {
                        let filter =
                            crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                        filtered
                            .into_iter()
                            .filter(|r| match r.point.payload.as_ref() {
                                Some(p) => filter.matches(p),
                                None => filter.matches(&serde_json::Value::Null),
                            })
                            .take(limit)
                            .collect()
                    } else {
                        filtered
                    }
                } else {
                    filtered
                }
            }
            // NEAR + similarity() + optional metadata: find candidates, then filter by ALL thresholds
            // Pattern: "Find top-k neighbors AND keep only those matching ALL similarity conditions"
            (Some(vector), Some((field, sim_vec, op, threshold)), filter_cond) => {
                // Validate field name - currently only "vector" is supported
                if field != "vector" {
                    return Err(crate::error::Error::Config(format!(
                        "similarity() field '{}' not found. Only 'vector' field is supported. \
                        Multi-vector support is planned for a future release.",
                        field
                    )));
                }

                // 1. NEAR finds candidates (overfetch for filtering headroom)
                let overfetch_factor = 10 * similarity_conditions.len().max(1);
                let candidates_k = limit.saturating_mul(overfetch_factor).min(MAX_LIMIT);
                let candidates = self.search(vector, candidates_k)?;

                // 2. EPIC-044 US-001: Apply ALL similarity filters sequentially (cascade)
                let filter_k = limit.saturating_mul(2);
                let mut filtered = self
                    .filter_by_similarity(candidates, field, sim_vec, *op, *threshold, filter_k);

                // Apply remaining similarity conditions
                for (sim_field, sim_vec, sim_op, sim_threshold) in
                    similarity_conditions.iter().skip(1)
                {
                    if sim_field != "vector" {
                        return Err(crate::error::Error::Config(format!(
                            "similarity() field '{}' not found. Only 'vector' field is supported.",
                            sim_field
                        )));
                    }
                    filtered = self.filter_by_similarity(
                        filtered,
                        sim_field,
                        sim_vec,
                        *sim_op,
                        *sim_threshold,
                        filter_k,
                    );
                }

                // 3. Apply additional metadata filters if present
                if let Some(cond) = filter_cond {
                    let metadata_filter = Self::extract_metadata_filter(cond);
                    if let Some(filter_cond) = metadata_filter {
                        let filter =
                            crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                        filtered
                            .into_iter()
                            .filter(|r| match r.point.payload.as_ref() {
                                Some(p) => filter.matches(p),
                                None => filter.matches(&serde_json::Value::Null),
                            })
                            .take(limit)
                            .collect()
                    } else {
                        filtered
                    }
                } else {
                    filtered
                }
            }
            (Some(vector), None, Some(ref cond)) => {
                // Check if condition contains MATCH for hybrid search
                if let Some(text_query) = Self::extract_match_query(cond) {
                    // Hybrid search: NEAR + MATCH
                    self.hybrid_search(vector, &text_query, limit, None)?
                } else {
                    // Vector search with metadata filter
                    let filter =
                        crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
                    self.search_with_filter(vector, limit, &filter)?
                }
            }
            (Some(vector), _, None) => {
                // Pure vector search
                if let Some(ef) = ef_search {
                    self.search_with_ef(vector, limit, ef)?
                } else {
                    self.search(vector, limit)?
                }
            }
            (None, None, Some(ref cond)) => {
                // Metadata-only filter (table scan + filter)
                // If it's a MATCH condition, use text search
                if let crate::velesql::Condition::Match(ref m) = cond {
                    // Pure text search - no filter needed
                    self.text_search(&m.query, limit)
                } else {
                    // Generic metadata filter: perform a scan (fallback)
                    let filter =
                        crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
                    self.execute_scan_query(&filter, limit)
                }
            }
            (None, None, None) => {
                // SELECT * FROM docs LIMIT N (no WHERE)
                self.execute_scan_query(
                    &crate::filter::Filter::new(crate::filter::Condition::And {
                        conditions: vec![],
                    }),
                    limit,
                )
            }
        };

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

        Ok(results)
    }

    // NOTE: apply_distinct and compute_distinct_key moved to distinct.rs
    // (EPIC-061/US-003 refactoring)

    // NOTE: filter_by_similarity, execute_not_similarity_query, extract_not_similarity_condition,
    // execute_scan_query moved to similarity_filter.rs (Plan 04-04)

    // NOTE: execute_union_query, matches_metadata_filter, split_or_condition_with_outer_filter
    // moved to union_query.rs (Plan 04-04)
}
