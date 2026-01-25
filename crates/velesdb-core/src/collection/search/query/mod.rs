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

mod extraction;
pub mod join;
mod ordering;
pub mod pushdown;
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

        // 1. Extract vector search (NEAR) or similarity() if present
        let mut vector_search = None;
        let mut similarity_condition = None;
        let mut filter_condition = None;

        if let Some(ref cond) = stmt.where_clause {
            // BUG-4 FIX: Validate query structure before extraction
            Self::validate_similarity_query_structure(cond)?;

            let mut extracted_cond = cond.clone();
            vector_search = self.extract_vector_search(&mut extracted_cond, params)?;
            similarity_condition = self.extract_similarity_condition(&extracted_cond, params)?;
            filter_condition = Some(extracted_cond);

            // NEAR + similarity() is supported: NEAR finds candidates, similarity() filters by threshold
            // This is a common pattern in RAG/agentic memory: find top-k AND filter by confidence
        }

        // 2. Resolve WITH clause options
        let mut ef_search = None;
        if let Some(ref with) = stmt.with_clause {
            ef_search = with.get_ef_search();
        }

        // 3. Execute query based on extracted components
        let mut results = match (&vector_search, &similarity_condition, &filter_condition) {
            // similarity() function - use vector to search, then filter by threshold
            // Also apply any additional metadata filters from the WHERE clause
            //
            // BUG-1/3 NOTE: This uses ANN (top-K) search, not exhaustive search.
            // Points outside the top-K window may match the threshold but won't be returned.
            // For exact threshold semantics, use a larger LIMIT or consider full scan.
            // We use a 10x over-fetch factor to reduce false negatives.
            (None, Some((field, vec, op, threshold)), filter_cond) => {
                // BUG-5 FIX: Validate field name - currently only "vector" is supported
                if field != "vector" {
                    return Err(crate::error::Error::Config(format!(
                        "similarity() field '{}' not found. Only 'vector' field is supported. \
                        Multi-vector support is planned for a future release.",
                        field
                    )));
                }

                // BUG-1/3 FIX: Increase over-fetch factor from 4x to 10x to reduce false negatives
                // This is a trade-off between performance and accuracy for threshold queries
                let candidates_k = limit.saturating_mul(10).min(MAX_LIMIT);
                let candidates = self.search(vec, candidates_k)?;

                // First filter by similarity threshold
                let filter_k = limit.saturating_mul(2);
                let similarity_filtered =
                    self.filter_by_similarity(candidates, field, vec, *op, *threshold, filter_k);

                // Then apply any additional metadata filters (e.g., AND category = 'tech')
                if let Some(cond) = filter_cond {
                    // Extract non-similarity parts of the condition for metadata filtering
                    let metadata_filter = Self::extract_metadata_filter(cond);
                    if let Some(filter_cond) = metadata_filter {
                        let filter =
                            crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                        // BUG-4 FIX: Don't drop points with null payload - check if filter matches null
                        similarity_filtered
                            .into_iter()
                            .filter(|r| match r.point.payload.as_ref() {
                                Some(p) => filter.matches(p),
                                None => filter.matches(&serde_json::Value::Null),
                            })
                            .take(limit)
                            .collect()
                    } else {
                        similarity_filtered
                    }
                } else {
                    similarity_filtered
                }
            }
            // NEAR + similarity() + optional metadata: find candidates, then filter by threshold
            // Pattern: "Find top-k neighbors AND keep only those with similarity > threshold"
            (Some(vector), Some((field, sim_vec, op, threshold)), filter_cond) => {
                // BUG-5 FIX: Validate field name - currently only "vector" is supported
                if field != "vector" {
                    return Err(crate::error::Error::Config(format!(
                        "similarity() field '{}' not found. Only 'vector' field is supported. \
                        Multi-vector support is planned for a future release.",
                        field
                    )));
                }

                // 1. NEAR finds candidates (overfetch for filtering headroom)
                // BUG-1/3 FIX: Increase over-fetch factor to 10x for better threshold accuracy
                let candidates_k = limit.saturating_mul(10).min(MAX_LIMIT);
                let candidates = self.search(vector, candidates_k)?;

                // 2. Apply similarity threshold filter
                let filter_k = limit.saturating_mul(2);
                let similarity_filtered = self
                    .filter_by_similarity(candidates, field, sim_vec, *op, *threshold, filter_k);

                // 3. Apply additional metadata filters if present
                if let Some(cond) = filter_cond {
                    let metadata_filter = Self::extract_metadata_filter(cond);
                    if let Some(filter_cond) = metadata_filter {
                        let filter =
                            crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                        // BUG-4 FIX: Don't drop points with null payload - check if filter matches null
                        similarity_filtered
                            .into_iter()
                            .filter(|r| match r.point.payload.as_ref() {
                                Some(p) => filter.matches(p),
                                None => filter.matches(&serde_json::Value::Null),
                            })
                            .take(limit)
                            .collect()
                    } else {
                        similarity_filtered
                    }
                } else {
                    similarity_filtered
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

        // Apply ORDER BY if present
        if let Some(ref order_by) = stmt.order_by {
            self.apply_order_by(&mut results, order_by, params)?;
        }

        // Apply limit
        results.truncate(limit);

        Ok(results)
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
                    // Update score to reflect the similarity() vector score
                    r.score = score;
                    Some(r)
                } else {
                    None
                }
            })
            .take(limit)
            .collect()
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
