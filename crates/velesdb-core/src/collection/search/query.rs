//! VelesQL query execution for Collection.
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

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};
use std::cmp::Ordering;

/// Compare two JSON values for sorting.
fn compare_json_values(a: Option<&serde_json::Value>, b: Option<&serde_json::Value>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => {
            // Compare by type priority: numbers, strings, booleans, null
            match (va, vb) {
                (serde_json::Value::Number(na), serde_json::Value::Number(nb)) => {
                    let fa = na.as_f64().unwrap_or(0.0);
                    let fb = nb.as_f64().unwrap_or(0.0);
                    fa.partial_cmp(&fb).unwrap_or(Ordering::Equal)
                }
                (serde_json::Value::String(sa), serde_json::Value::String(sb)) => sa.cmp(sb),
                (serde_json::Value::Bool(ba), serde_json::Value::Bool(bb)) => ba.cmp(bb),
                _ => Ordering::Equal,
            }
        }
    }
}

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
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let stmt = &query.select;
        let limit = usize::try_from(stmt.limit.unwrap_or(10)).unwrap_or(usize::MAX);

        // 1. Extract vector search (NEAR) or similarity() if present
        let mut vector_search = None;
        let mut similarity_condition = None;
        let mut filter_condition = None;

        if let Some(ref cond) = stmt.where_clause {
            let mut extracted_cond = cond.clone();
            vector_search = self.extract_vector_search(&mut extracted_cond, params)?;
            similarity_condition = self.extract_similarity_condition(&extracted_cond, params)?;
            filter_condition = Some(extracted_cond);
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
            (None, Some((field, vec, op, threshold)), filter_cond) => {
                // Get more candidates for filtering (both similarity and metadata)
                let candidates = self.search(vec, limit * 4)?;

                // First filter by similarity threshold
                let similarity_filtered =
                    self.filter_by_similarity(candidates, field, vec, *op, *threshold, limit * 2);

                // Then apply any additional metadata filters (e.g., AND category = 'tech')
                if let Some(cond) = filter_cond {
                    // Extract non-similarity parts of the condition for metadata filtering
                    let metadata_filter = Self::extract_metadata_filter(cond);
                    if let Some(filter_cond) = metadata_filter {
                        let filter =
                            crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                        similarity_filtered
                            .into_iter()
                            .filter(|r| r.point.payload.as_ref().is_some_and(|p| filter.matches(p)))
                            .take(limit)
                            .collect()
                    } else {
                        similarity_filtered
                    }
                } else {
                    similarity_filtered
                }
            }
            (Some(vector), _, Some(ref cond)) => {
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

    /// Apply ORDER BY clause to results.
    fn apply_order_by(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        use crate::velesql::OrderByExpr;

        if order_by.is_empty() {
            return Ok(());
        }

        // For now, we only support single ORDER BY expression
        // Multiple ORDER BY items would require more complex sorting
        let first = &order_by[0];

        match &first.expr {
            OrderByExpr::Similarity(sim) => {
                // Sort by similarity score
                // User expectation: DESC = most similar first, ASC = least similar first
                let user_wants_most_similar_first = first.descending;

                // If the similarity vector is different from the search vector,
                // we need to recompute scores
                let order_vec = self.resolve_vector(&sim.vector, params)?;

                // Recompute metric scores for accurate ordering
                for result in results.iter_mut() {
                    let score = self.compute_metric_score(&result.point.vector, &order_vec);
                    result.score = score;
                }

                // Adjust sort order based on metric semantics:
                // - Similarity metrics (Cosine, DotProduct, Jaccard): higher = more similar
                // - Distance metrics (Euclidean, Hamming): lower = more similar
                let metric = self.config.read().metric;
                let sort_descending = if metric.higher_is_better() {
                    // Higher score = more similar, so DESC for most similar
                    user_wants_most_similar_first
                } else {
                    // Lower score = more similar, so ASC for most similar
                    !user_wants_most_similar_first
                };

                if sort_descending {
                    results.sort_by(|a, b| {
                        b.score
                            .partial_cmp(&a.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                } else {
                    results.sort_by(|a, b| {
                        a.score
                            .partial_cmp(&b.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }
            OrderByExpr::Field(field_name) => {
                // Sort by payload field value
                let descending = first.descending;

                results.sort_by(|a, b| {
                    let val_a = a.point.payload.as_ref().and_then(|p| p.get(field_name));
                    let val_b = b.point.payload.as_ref().and_then(|p| p.get(field_name));

                    let cmp = compare_json_values(val_a, val_b);
                    if descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
        }

        Ok(())
    }

    /// Resolve a vector expression to actual vector values.
    fn resolve_vector(
        &self,
        vector: &crate::velesql::VectorExpr,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<f32>> {
        use crate::velesql::VectorExpr;

        match vector {
            VectorExpr::Literal(v) => Ok(v.clone()),
            VectorExpr::Parameter(name) => {
                let val = params
                    .get(name)
                    .ok_or_else(|| Error::Config(format!("Missing query parameter: ${name}")))?;
                if let serde_json::Value::Array(arr) = val {
                    #[allow(clippy::cast_possible_truncation)]
                    arr.iter()
                        .map(|v| {
                            v.as_f64().map(|f| f as f32).ok_or_else(|| {
                                Error::Config(format!(
                                    "Invalid vector parameter ${name}: expected numbers"
                                ))
                            })
                        })
                        .collect::<Result<Vec<f32>>>()
                } else {
                    Err(Error::Config(format!(
                        "Invalid vector parameter ${name}: expected array"
                    )))
                }
            }
        }
    }

    /// Compute the metric score between two vectors using the collection's configured metric.
    ///
    /// **Note:** This returns the raw metric score, not a normalized similarity.
    /// The interpretation depends on the metric:
    /// - **Cosine**: Returns cosine similarity (higher = more similar)
    /// - **DotProduct**: Returns dot product (higher = more similar)
    /// - **Euclidean**: Returns euclidean distance (lower = more similar)
    /// - **Hamming**: Returns hamming distance (lower = more similar)
    /// - **Jaccard**: Returns jaccard similarity (higher = more similar)
    ///
    /// Use `metric.higher_is_better()` to determine score interpretation.
    fn compute_metric_score(&self, a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        // Use the collection's configured metric for consistent behavior
        let metric = self.config.read().metric;
        metric.calculate(a, b)
    }

    /// Helper to extract MATCH query from any nested condition.
    fn extract_match_query(condition: &crate::velesql::Condition) -> Option<String> {
        use crate::velesql::Condition;
        match condition {
            Condition::Match(m) => Some(m.query.clone()),
            Condition::And(left, right) => {
                Self::extract_match_query(left).or_else(|| Self::extract_match_query(right))
            }
            Condition::Group(inner) => Self::extract_match_query(inner),
            _ => None,
        }
    }

    /// Internal helper to extract vector search from WHERE clause.
    #[allow(clippy::self_only_used_in_recursion)]
    fn extract_vector_search(
        &self,
        condition: &mut crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<Vec<f32>>> {
        use crate::velesql::{Condition, VectorExpr};

        match condition {
            Condition::VectorSearch(vs) => {
                let vec = match &vs.vector {
                    VectorExpr::Literal(v) => v.clone(),
                    VectorExpr::Parameter(name) => {
                        let val = params.get(name).ok_or_else(|| {
                            Error::Config(format!("Missing query parameter: ${name}"))
                        })?;
                        if let serde_json::Value::Array(arr) = val {
                            #[allow(clippy::cast_possible_truncation)]
                            arr.iter()
                                .map(|v| {
                                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                                        Error::Config(format!(
                                            "Invalid vector parameter ${name}: expected numbers"
                                        ))
                                    })
                                })
                                .collect::<Result<Vec<f32>>>()?
                        } else {
                            return Err(Error::Config(format!(
                                "Invalid vector parameter ${name}: expected array"
                            )));
                        }
                    }
                };
                Ok(Some(vec))
            }
            Condition::And(left, right) => {
                if let Some(v) = self.extract_vector_search(left, params)? {
                    return Ok(Some(v));
                }
                self.extract_vector_search(right, params)
            }
            Condition::Group(inner) => self.extract_vector_search(inner, params),
            _ => Ok(None),
        }
    }

    /// Extract similarity condition from WHERE clause.
    /// Returns (field, vector, operator, threshold) if found.
    #[allow(clippy::type_complexity)]
    #[allow(clippy::only_used_in_recursion)]
    #[allow(clippy::self_only_used_in_recursion)]
    fn extract_similarity_condition(
        &self,
        condition: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<(String, Vec<f32>, crate::velesql::CompareOp, f64)>> {
        use crate::velesql::{Condition, VectorExpr};

        match condition {
            Condition::Similarity(sim) => {
                let vec = match &sim.vector {
                    VectorExpr::Literal(v) => v.clone(),
                    VectorExpr::Parameter(name) => {
                        let val = params.get(name).ok_or_else(|| {
                            Error::Config(format!("Missing query parameter: ${name}"))
                        })?;
                        if let serde_json::Value::Array(arr) = val {
                            #[allow(clippy::cast_possible_truncation)]
                            arr.iter()
                                .map(|v| {
                                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                                        Error::Config(format!(
                                            "Invalid vector parameter ${name}: expected numbers"
                                        ))
                                    })
                                })
                                .collect::<Result<Vec<f32>>>()?
                        } else {
                            return Err(Error::Config(format!(
                                "Invalid vector parameter ${name}: expected array"
                            )));
                        }
                    }
                };
                Ok(Some((sim.field.clone(), vec, sim.operator, sim.threshold)))
            }
            Condition::And(left, right) => {
                if let Some(s) = self.extract_similarity_condition(left, params)? {
                    return Ok(Some(s));
                }
                self.extract_similarity_condition(right, params)
            }
            Condition::Group(inner) => self.extract_similarity_condition(inner, params),
            _ => Ok(None),
        }
    }

    /// Extract non-similarity parts of a condition for metadata filtering.
    ///
    /// This removes `SimilarityFilter` conditions from the tree and returns
    /// only the metadata filter parts (e.g., `category = 'tech'`).
    fn extract_metadata_filter(
        condition: &crate::velesql::Condition,
    ) -> Option<crate::velesql::Condition> {
        use crate::velesql::Condition;

        match condition {
            // Remove vector search conditions - they're handled separately by the query executor
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_) => None,
            // For AND: keep both sides if they exist, or just one side
            Condition::And(left, right) => {
                let left_filter = Self::extract_metadata_filter(left);
                let right_filter = Self::extract_metadata_filter(right);
                match (left_filter, right_filter) {
                    (Some(l), Some(r)) => Some(Condition::And(Box::new(l), Box::new(r))),
                    (Some(l), None) => Some(l),
                    (None, Some(r)) => Some(r),
                    (None, None) => None,
                }
            }
            // For OR: both sides must exist
            Condition::Or(left, right) => {
                let left_filter = Self::extract_metadata_filter(left);
                let right_filter = Self::extract_metadata_filter(right);
                match (left_filter, right_filter) {
                    (Some(l), Some(r)) => Some(Condition::Or(Box::new(l), Box::new(r))),
                    _ => None, // OR requires both sides
                }
            }
            // Unwrap groups
            Condition::Group(inner) => {
                Self::extract_metadata_filter(inner).map(|c| Condition::Group(Box::new(c)))
            }
            // Keep all other conditions (comparisons, IN, BETWEEN, etc.)
            other => Some(other.clone()),
        }
    }

    /// Filter search results by similarity threshold.
    ///
    /// For similarity() function queries, we need to check if results meet the threshold.
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
        _query_vec: &[f32],
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
            .filter(|r| {
                let score = r.score;
                // For distance metrics, invert comparisons so "similarity > X" means "distance < X"
                if higher_is_better {
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
                        CompareOp::Gt => score < threshold_f32,   // more similar = lower distance
                        CompareOp::Gte => score <= threshold_f32,
                        CompareOp::Lt => score > threshold_f32,   // less similar = higher distance
                        CompareOp::Lte => score >= threshold_f32,
                        CompareOp::Eq => (score - threshold_f32).abs() < 0.001,
                        CompareOp::NotEq => (score - threshold_f32).abs() >= 0.001,
                    }
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
