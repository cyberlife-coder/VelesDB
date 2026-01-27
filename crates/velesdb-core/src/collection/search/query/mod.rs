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
//! TODO: Integrate `QueryPlanner::choose_hybrid_strategy()` into `execute_query()`
//! to leverage cost-based optimization for complex queries.

mod aggregation;
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
mod validation;

// Re-export for potential external use
#[allow(unused_imports)]
pub use ordering::compare_json_values;
// Re-export join functions for future integration with execute_query
#[allow(unused_imports)]
pub use join::{execute_join, JoinedResult};

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

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
            results = Self::apply_distinct(results, &stmt.columns);
        }

        // Apply ORDER BY if present
        if let Some(ref order_by) = stmt.order_by {
            self.apply_order_by(&mut results, order_by, params)?;
        }

        // Apply limit
        results.truncate(limit);

        Ok(results)
    }

    /// Apply DISTINCT deduplication to results based on selected columns (EPIC-052 US-001).
    ///
    /// Uses HashSet for O(n) complexity and preserves insertion order.
    fn apply_distinct(
        results: Vec<SearchResult>,
        columns: &crate::velesql::SelectColumns,
    ) -> Vec<SearchResult> {
        use rustc_hash::FxHashSet;

        // If SELECT *, deduplicate by all payload fields
        let column_names: Vec<String> = match columns {
            crate::velesql::SelectColumns::Columns(cols) => {
                cols.iter().map(|c| c.name.clone()).collect()
            }
            crate::velesql::SelectColumns::Mixed { columns: cols, .. } => {
                cols.iter().map(|c| c.name.clone()).collect()
            }
            // All or Aggregations: use full payload or no deduplication
            crate::velesql::SelectColumns::All | crate::velesql::SelectColumns::Aggregations(_) => {
                Vec::new()
            }
        };

        let mut seen: FxHashSet<String> = FxHashSet::default();
        results
            .into_iter()
            .filter(|r| {
                let key = Self::compute_distinct_key(r, &column_names);
                seen.insert(key)
            })
            .collect()
    }

    /// Compute a unique key for DISTINCT deduplication.
    fn compute_distinct_key(result: &SearchResult, columns: &[String]) -> String {
        let payload = result.point.payload.as_ref();

        if columns.is_empty() {
            // SELECT * or SELECT DISTINCT *: use full payload as key
            payload.map_or_else(|| "null".to_string(), ToString::to_string)
        } else {
            // SELECT DISTINCT col1, col2: use specific columns
            columns
                .iter()
                .map(|col| {
                    payload
                        .and_then(|p| p.get(col))
                        .map_or_else(|| "null".to_string(), ToString::to_string)
                })
                .collect::<Vec<_>>()
                .join("\x1F") // ASCII Unit Separator
        }
    }

    /// Filter search results by similarity threshold.
    ///
    /// For similarity() function queries, we need to check if results meet the threshold.
    ///
    /// **BUG-2 FIX:** Recomputes similarity using `query_vec`, not the cached NEAR scores.
    /// This is critical when NEAR and similarity() use different vectors.
    ///
    /// **Metric-aware semantics:**
    /// - For similarity metrics (Cosine, DotProduct, Jaccard): higher score = more similar
    ///   - `similarity() > 0.8` keeps results with score > 0.8
    /// - For distance metrics (Euclidean, Hamming): lower score = more similar
    ///   - `similarity() > 0.8` is interpreted as "more similar than threshold"
    ///   - which means distance < 0.8 (comparison inverted)
    #[allow(clippy::too_many_arguments)]
    fn filter_by_similarity(
        &self,
        candidates: Vec<SearchResult>,
        _field: &str,
        query_vec: &[f32],
        op: crate::velesql::CompareOp,
        threshold: f64,
        limit: usize,
    ) -> Vec<SearchResult> {
        use crate::velesql::CompareOp;

        let config = self.config.read();
        let higher_is_better = config.metric.higher_is_better();
        drop(config);

        let threshold_f32 = threshold as f32;

        candidates
            .into_iter()
            .filter_map(|mut r| {
                // BUG-2 FIX: Recompute similarity using the similarity() vector, not NEAR scores
                // This ensures correct filtering when NEAR and similarity() use different vectors
                let score = self.compute_metric_score(&r.point.vector, query_vec);

                // For distance metrics, invert comparisons so "similarity > X" means "distance < X"
                let passes = if higher_is_better {
                    // Similarity metrics: direct comparison
                    match op {
                        CompareOp::Gt => score > threshold_f32,
                        CompareOp::Gte => score >= threshold_f32,
                        CompareOp::Lt => score < threshold_f32,
                        CompareOp::Lte => score <= threshold_f32,
                        CompareOp::Eq => (score - threshold_f32).abs() < 0.001,
                        CompareOp::NotEq => (score - threshold_f32).abs() >= 0.001,
                    }
                } else {
                    // Distance metrics: inverted comparison
                    // "similarity > X" means "more similar than X" = "distance < X"
                    match op {
                        CompareOp::Gt => score < threshold_f32, // more similar = lower distance
                        CompareOp::Gte => score <= threshold_f32,
                        CompareOp::Lt => score > threshold_f32, // less similar = higher distance
                        CompareOp::Lte => score >= threshold_f32,
                        CompareOp::Eq => (score - threshold_f32).abs() < 0.001,
                        CompareOp::NotEq => (score - threshold_f32).abs() >= 0.001,
                    }
                };

                if passes {
                    // EPIC-044 US-001: Update score to reflect THIS similarity filter's score.
                    // When multiple similarity() conditions are used (cascade filtering),
                    // the final score will be from the LAST filter applied.
                    // This is intentional: each filter re-scores against its vector.
                    r.score = score;
                    Some(r)
                } else {
                    None
                }
            })
            .take(limit)
            .collect()
    }

    /// EPIC-044 US-002: Execute union query for similarity() OR metadata patterns.
    ///
    /// This method handles queries like:
    /// `WHERE similarity(v, $v) > 0.8 OR category = 'tech'`
    ///
    /// Issue #122: Also handles nested patterns like:
    /// `WHERE (similarity(v, $v) > 0.8 OR category = 'tech') AND status = 'active'`
    ///
    /// It executes:
    /// 1. Vector search for similarity matches
    /// 2. Metadata scan for non-similarity matches
    /// 3. Apply outer AND filters to both result sets
    /// 4. Merges results with deduplication (by point ID)
    ///
    /// Scoring:
    /// - Similarity matches: use similarity score
    /// - Metadata-only matches: use score 1.0
    /// - Both matching: use similarity score (higher priority)
    fn execute_union_query(
        &self,
        condition: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        use std::collections::HashMap;

        // Issue #122: Extract similarity, metadata, AND outer filter from condition
        let (similarity_cond, metadata_cond, outer_filter) =
            Self::split_or_condition_with_outer_filter(condition);

        let mut results_map: HashMap<u64, SearchResult> = HashMap::new();

        // 1. Execute similarity search if we have a similarity condition
        if let Some(sim_cond) = similarity_cond {
            let similarity_conditions =
                self.extract_all_similarity_conditions(&sim_cond, params)?;
            if let Some((field, vec, op, threshold)) = similarity_conditions.first() {
                if field != "vector" {
                    return Err(crate::error::Error::Config(format!(
                        "similarity() field '{}' not found. Only 'vector' field is supported.",
                        field
                    )));
                }

                let overfetch_factor = 10;
                let candidates_k = limit.saturating_mul(overfetch_factor).min(MAX_LIMIT);
                let candidates = self.search(vec, candidates_k)?;

                let filter_k = limit.saturating_mul(2);
                let filtered =
                    self.filter_by_similarity(candidates, field, vec, *op, *threshold, filter_k);

                for result in filtered {
                    // Issue #122: Apply outer filter to similarity results
                    if let Some(ref outer) = outer_filter {
                        if !self.matches_metadata_filter(&result.point, outer) {
                            continue;
                        }
                    }
                    results_map.insert(result.point.id, result);
                }
            }
        }

        // 2. Execute metadata scan if we have a metadata condition
        if let Some(meta_cond) = metadata_cond {
            // Issue #122: Combine metadata condition with outer filter
            let combined_cond = match outer_filter {
                Some(ref outer) => {
                    crate::velesql::Condition::And(Box::new(meta_cond), Box::new(outer.clone()))
                }
                None => meta_cond,
            };
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(combined_cond));
            let metadata_results = self.execute_scan_query(&filter, limit);

            for result in metadata_results {
                // Only add if not already found by similarity search
                // If already present, keep the similarity score (higher priority)
                results_map.entry(result.point.id).or_insert(result);
            }
        }

        // 3. Collect and return results
        let mut results: Vec<SearchResult> = results_map.into_values().collect();

        // Sort by score descending (similarity matches first)
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }

    /// Check if a point matches a metadata filter condition.
    /// Used for applying outer AND filters to similarity results.
    fn matches_metadata_filter(
        &self,
        point: &crate::Point,
        condition: &crate::velesql::Condition,
    ) -> bool {
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(condition.clone()));
        match point.payload.as_ref() {
            Some(payload) => filter.matches(payload),
            None => false, // No payload means filter doesn't match
        }
    }

    /// Split an OR condition into similarity and metadata parts, extracting outer AND filters.
    ///
    /// For `similarity() > 0.8 OR category = 'tech'`, returns:
    /// - similarity_cond: Some(similarity() > 0.8)
    /// - metadata_cond: Some(category = 'tech')
    /// - outer_filter: None
    ///
    /// For `(similarity() > 0.8 OR category = 'tech') AND status = 'active'`, returns:
    /// - similarity_cond: Some(similarity() > 0.8)
    /// - metadata_cond: Some(category = 'tech')
    /// - outer_filter: Some(status = 'active')
    ///
    /// Issue #122: Handle nested AND/OR patterns correctly.
    fn split_or_condition_with_outer_filter(
        condition: &crate::velesql::Condition,
    ) -> (
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
        Option<crate::velesql::Condition>,
    ) {
        match condition {
            crate::velesql::Condition::Or(left, right) => {
                // Direct OR at top level
                let left_has_sim = Self::count_similarity_conditions(left) > 0;
                let right_has_sim = Self::count_similarity_conditions(right) > 0;

                match (left_has_sim, right_has_sim) {
                    (true, false) => (Some((**left).clone()), Some((**right).clone()), None),
                    (false, true) => (Some((**right).clone()), Some((**left).clone()), None),
                    _ => (Some(condition.clone()), None, None),
                }
            }
            crate::velesql::Condition::And(left, right) => {
                // Issue #122: Check if one side contains an OR with similarity
                let left_has_problematic_or = Self::has_similarity_in_problematic_or(left);
                let right_has_problematic_or = Self::has_similarity_in_problematic_or(right);

                match (left_has_problematic_or, right_has_problematic_or) {
                    (true, false) => {
                        // Left has the OR, right is an outer filter
                        let (sim, meta, inner_filter) =
                            Self::split_or_condition_with_outer_filter(left);
                        // Combine inner_filter with right as outer filter
                        let outer = match inner_filter {
                            Some(inner) => Some(crate::velesql::Condition::And(
                                Box::new(inner),
                                Box::new((**right).clone()),
                            )),
                            None => Some((**right).clone()),
                        };
                        (sim, meta, outer)
                    }
                    (false, true) => {
                        // Right has the OR, left is an outer filter
                        let (sim, meta, inner_filter) =
                            Self::split_or_condition_with_outer_filter(right);
                        let outer = match inner_filter {
                            Some(inner) => Some(crate::velesql::Condition::And(
                                Box::new((**left).clone()),
                                Box::new(inner),
                            )),
                            None => Some((**left).clone()),
                        };
                        (sim, meta, outer)
                    }
                    _ => {
                        // Both or neither - treat as before
                        if Self::count_similarity_conditions(condition) > 0 {
                            (Some(condition.clone()), None, None)
                        } else {
                            (None, Some(condition.clone()), None)
                        }
                    }
                }
            }
            crate::velesql::Condition::Group(inner) => {
                // Unwrap group and recurse
                Self::split_or_condition_with_outer_filter(inner)
            }
            // Not an OR or AND condition - treat as similarity if it contains similarity
            _ => {
                if Self::count_similarity_conditions(condition) > 0 {
                    (Some(condition.clone()), None, None)
                } else {
                    (None, Some(condition.clone()), None)
                }
            }
        }
    }

    /// EPIC-044 US-003: Execute NOT similarity() query via full scan.
    ///
    /// This method handles queries like:
    /// `WHERE NOT similarity(v, $v) > 0.8`
    /// Which is equivalent to: `WHERE similarity(v, $v) <= 0.8`
    ///
    /// **Performance Warning**: This requires scanning ALL documents.
    /// Always use with LIMIT for acceptable performance.
    fn execute_not_similarity_query(
        &self,
        condition: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Extract the NOT similarity condition
        let (sim_field, sim_vec, sim_op, sim_threshold) =
            self.extract_not_similarity_condition(condition, params)?;

        // Validate field
        if sim_field != "vector" {
            return Err(crate::error::Error::Config(format!(
                "similarity() field '{}' not found. Only 'vector' field is supported.",
                sim_field
            )));
        }

        // Log performance warning for large collections
        let vector_storage = self.vector_storage.read();
        let total_count = vector_storage.ids().len();
        drop(vector_storage);

        if total_count > 10_000 && limit > 1000 {
            tracing::warn!(
                "NOT similarity() query scanning {} documents with LIMIT {}. \
                Consider using a smaller LIMIT for better performance.",
                total_count,
                limit
            );
        }

        // PR #120 Review Fix: Extract metadata filter for AND conditions
        // e.g., WHERE NOT similarity(v, $v) > 0.8 AND category = 'tech'
        let metadata_filter = Self::extract_metadata_filter(condition);
        let filter = metadata_filter
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond)));

        // Full scan with similarity exclusion + metadata filter
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let config = self.config.read();
        let higher_is_better = config.metric.higher_is_better();
        drop(config);

        let threshold_f32 = sim_threshold as f32;
        let mut results = Vec::new();

        for id in vector_storage.ids() {
            if let Ok(Some(vector)) = vector_storage.retrieve(id) {
                // Compute similarity score
                let score = self.compute_metric_score(&vector, &sim_vec);

                // Invert the condition: NOT (similarity > threshold) = similarity <= threshold
                let excluded = if higher_is_better {
                    match sim_op {
                        crate::velesql::CompareOp::Gt => score > threshold_f32,
                        crate::velesql::CompareOp::Gte => score >= threshold_f32,
                        crate::velesql::CompareOp::Lt => score < threshold_f32,
                        crate::velesql::CompareOp::Lte => score <= threshold_f32,
                        crate::velesql::CompareOp::Eq => (score - threshold_f32).abs() < 0.001,
                        crate::velesql::CompareOp::NotEq => (score - threshold_f32).abs() >= 0.001,
                    }
                } else {
                    // Distance metrics: inverted
                    match sim_op {
                        crate::velesql::CompareOp::Gt => score < threshold_f32,
                        crate::velesql::CompareOp::Gte => score <= threshold_f32,
                        crate::velesql::CompareOp::Lt => score > threshold_f32,
                        crate::velesql::CompareOp::Lte => score >= threshold_f32,
                        crate::velesql::CompareOp::Eq => (score - threshold_f32).abs() < 0.001,
                        crate::velesql::CompareOp::NotEq => (score - threshold_f32).abs() >= 0.001,
                    }
                };

                // Include if NOT excluded by similarity
                if !excluded {
                    let payload = payload_storage.retrieve(id).ok().flatten();

                    // PR #120 Review Fix: Apply metadata filter if present
                    let matches_metadata = match (&filter, &payload) {
                        (Some(f), Some(p)) => f.matches(p),
                        (Some(f), None) => f.matches(&serde_json::Value::Null),
                        (None, _) => true, // No metadata filter = match all
                    };

                    if matches_metadata {
                        results.push(SearchResult::new(
                            Point {
                                id,
                                vector,
                                payload,
                            },
                            score,
                        ));

                        if results.len() >= limit {
                            break;
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Extract similarity condition from inside a NOT clause.
    fn extract_not_similarity_condition(
        &self,
        condition: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(String, Vec<f32>, crate::velesql::CompareOp, f64)> {
        match condition {
            crate::velesql::Condition::Not(inner) => {
                // Extract from inside NOT
                let conditions = self.extract_all_similarity_conditions(inner, params)?;
                conditions.into_iter().next().ok_or_else(|| {
                    crate::error::Error::Config(
                        "NOT clause does not contain a similarity condition".to_string(),
                    )
                })
            }
            crate::velesql::Condition::And(left, right) => {
                // Try left, then right
                self.extract_not_similarity_condition(left, params)
                    .or_else(|_| self.extract_not_similarity_condition(right, params))
            }
            _ => Err(crate::error::Error::Config(
                "Expected NOT similarity() condition".to_string(),
            )),
        }
    }

    /// Fallback method for metadata-only queries without vector search.
    fn execute_scan_query(
        &self,
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> Vec<SearchResult> {
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();

        // Scan all points (slow fallback)
        // In production, this should use metadata indexes
        let mut results = Vec::new();

        // We need all IDs to scan
        let ids = vector_storage.ids();

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();
            let matches = match payload {
                Some(ref p) => filter.matches(p),
                None => filter.matches(&serde_json::Value::Null),
            };

            if matches {
                if let Ok(Some(vector)) = vector_storage.retrieve(id) {
                    results.push(SearchResult::new(
                        Point {
                            id,
                            vector,
                            payload,
                        },
                        1.0, // Constant score for scans
                    ));
                }
            }

            if results.len() >= limit {
                break;
            }
        }

        results
    }
}
