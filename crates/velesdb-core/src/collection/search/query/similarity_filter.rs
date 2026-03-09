//! Similarity filtering, NOT-similarity queries, and scan fallbacks.
//!
//! Extracted from query/mod.rs for complexity reduction (EPIC-044).

// SAFETY: Numeric casts in similarity filtering are intentional:
// - f64->f32 for similarity thresholds: precision loss acceptable for filtering

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
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
    pub(crate) fn filter_by_similarity(
        &self,
        candidates: Vec<SearchResult>,
        field: &str,
        query_vec: &[f32],
        op: crate::velesql::CompareOp,
        threshold: f64,
        limit: usize,
    ) -> Vec<SearchResult> {
        use crate::velesql::CompareOp;

        let config = self.config.read();
        let higher_is_better = config.metric.higher_is_better();
        drop(config);

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        // Reason: threshold is a user-provided f64 similarity score in [0.0, 1.0] range;
        // precision loss and truncation to f32 are acceptable for comparison purposes.
        let threshold_f32 = threshold as f32;

        candidates
            .into_iter()
            .filter_map(|mut r| {
                // Multi-vector support (P1-A): use named payload vector if field != "vector".
                // For the primary "vector" field, use the pre-loaded r.point.vector.
                let candidate_vec: std::borrow::Cow<[f32]> = if field == "vector" {
                    std::borrow::Cow::Borrowed(&r.point.vector)
                } else {
                    // Retrieve named vector from payload — skip on missing, warn on error.
                    match self.get_vector_for_field(r.point.id, field) {
                        Ok(Some(v)) => std::borrow::Cow::Owned(v),
                        Ok(None) => return None,
                        Err(e) => {
                            tracing::warn!(
                                point_id = r.point.id,
                                field = field,
                                error = %e,
                                "filter_by_similarity: failed to retrieve named vector field; point skipped"
                            );
                            return None;
                        }
                    }
                };

                // BUG-2 FIX: Recompute similarity using the similarity() vector, not NEAR scores
                // This ensures correct filtering when NEAR and similarity() use different vectors
                let score = self.compute_metric_score(&candidate_vec, query_vec);

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

    /// EPIC-044 US-003: Execute NOT similarity() query via full scan.
    ///
    /// This method handles queries like:
    /// `WHERE NOT similarity(v, $v) > 0.8`
    /// Which is equivalent to: `WHERE similarity(v, $v) <= 0.8`
    ///
    /// **Performance Warning**: This requires scanning ALL documents.
    /// Always use with LIMIT for acceptable performance.
    #[allow(clippy::too_many_lines)] // Reason: function is 81 lines, 1 over limit; extraction would split tightly coupled scan+filter logic
    pub(crate) fn execute_not_similarity_query(
        &self,
        condition: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Extract the NOT similarity condition
        let (sim_field, sim_vec, sim_op, sim_threshold) =
            self.extract_not_similarity_condition(condition, params)?;

        // Multi-vector (P1-A): field may be "vector" or a named payload vector.
        // get_vector_for_field handles both cases; named fields are validated per-point below.

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

        // Full scan with similarity exclusion + metadata filter.
        // For named payload vectors we must not hold vector_storage while calling
        // get_vector_for_field (which acquires payload_storage internally), so we
        // collect the IDs first, then drop the lock before iterating.
        let config = self.config.read();
        let higher_is_better = config.metric.higher_is_better();
        drop(config);

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        // Reason: sim_threshold is a user-provided f64 similarity score in [0.0, 1.0] range;
        // precision loss and truncation to f32 are acceptable for comparison purposes.
        let threshold_f32 = sim_threshold as f32;
        let mut results = Vec::new();

        // Collect all IDs upfront so we hold no read locks during the main loop.
        // get_vector_for_field and payload_storage.retrieve both acquire their own locks,
        // so we must not hold any of those guards here.
        let all_ids: Vec<u64> = self.vector_storage.read().ids().into_iter().collect();

        for id in all_ids {
            // Multi-vector (P1-A): use get_vector_for_field for both "vector" and named fields.
            let candidate_vec: Vec<f32> = match self.get_vector_for_field(id, &sim_field) {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(
                        point_id = id,
                        field = %sim_field,
                        error = %e,
                        "execute_not_similarity_query: failed to retrieve vector field; point skipped"
                    );
                    continue;
                }
            };
            let vector = candidate_vec;
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
                let payload = self.payload_storage.read().retrieve(id).ok().flatten();

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
                            sparse_vectors: None,
                        },
                        score,
                    ));

                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }

        Ok(results)
    }

    /// Extract similarity condition from inside a NOT clause.
    pub(crate) fn extract_not_similarity_condition(
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
    pub(crate) fn execute_scan_query(
        &self,
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> Vec<SearchResult> {
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();

        // Scan all points (slow fallback)
        // In production, this should use metadata indexes
        let mut results = Vec::new();

        // Prefer vector_storage IDs (vector collections). Fall back to payload_storage
        // IDs for metadata-only collections where vector_storage is empty.
        let vector_ids = vector_storage.ids();
        let ids: Vec<u64> = if vector_ids.is_empty() {
            payload_storage.ids()
        } else {
            vector_ids
        };

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();
            let matches = match payload {
                Some(ref p) => filter.matches(p),
                None => filter.matches(&serde_json::Value::Null),
            };

            if matches {
                // For metadata-only collections, vectors are empty (metadata stored
                // in payload_storage only). Use unwrap_or_default so both vector and
                // metadata-only points are included in scan results.
                let vector = vector_storage
                    .retrieve(id)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                results.push(SearchResult::new(
                    Point {
                        id,
                        vector,
                        payload,
                        sparse_vectors: None,
                    },
                    1.0, // Constant score for scans
                ));
            }

            if results.len() >= limit {
                break;
            }
        }

        results
    }

    /// Scans all metadata-matching points and rescores them by exact vector similarity.
    ///
    /// Used by the `GraphFirst` CBO strategy when the metadata filter is **highly selective**
    /// (eliminates most of the collection). In that case the ANN index over-fetch approach
    /// (`VectorFirst`) is suboptimal: a large `cbo_search_k` fetches many candidates that the
    /// filter will discard anyway.
    ///
    /// `GraphFirst` inverts the order:
    /// 1. Full-scan filtered by metadata → produces a *small* candidate set.
    /// 2. Exact vector similarity is computed for each candidate using the stored vector.
    /// 3. Results are sorted by score and truncated to `limit`.
    ///
    /// # Performance characteristic
    ///
    /// O(n_filtered) comparisons instead of O(k × over_fetch) ANN candidates —
    /// optimal when `n_filtered << k × over_fetch`.
    ///
    /// # SecDev
    ///
    /// All cosine scores are clamped to `[-1.0, 1.0]` to prevent silent NaN propagation
    /// from zero-norm vectors.
    pub(crate) fn scan_and_score_by_vector(
        &self,
        metadata_filter: &crate::filter::Filter,
        query: &[f32],
        limit: usize,
    ) -> Vec<SearchResult> {
        // MAX_LIMIT inline: same bound as the constant in query/mod.rs.
        const SCAN_CAP: usize = 100_000;

        let metric = self.config().metric;

        let mut candidates = self.execute_scan_query(metadata_filter, SCAN_CAP);

        for r in &mut candidates {
            // Exact distance computation using the stored vector.
            // Clamp is mandatory (SecDev) to guard against NaN from zero-norm vectors.
            r.score = metric.calculate(&r.point.vector, query).clamp(-1.0, 1.0);
        }

        if metric.higher_is_better() {
            candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        } else {
            candidates.sort_by(|a, b| a.score.total_cmp(&b.score));
        }

        candidates.truncate(limit);
        candidates
    }
}
