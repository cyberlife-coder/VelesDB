//! Similarity filtering, NOT-similarity queries, and scan fallbacks.
//!
//! Extracted from query/mod.rs for complexity reduction (EPIC-044).

// Reason: Numeric casts in similarity filtering are intentional:
// - f64->f32 for similarity thresholds: precision loss acceptable for filtering

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

/// Outcome of a tracked metadata scan (WO-D2): the collected matches plus
/// how many candidate ids were never visited because the scan already
/// reached `limit`.
///
/// `unscanned_ids > 0` means the scan stopped early; the unvisited ids are
/// an upper bound on the matches missed. Callers that use `limit` as a hard
/// scan cap (e.g. [`Collection::scan_and_score_by_vector`]) rely on it to
/// detect and report silent truncation.
pub(crate) struct TrackedScan {
    /// Matches collected before the scan stopped (`len() <= limit`).
    pub(crate) results: Vec<SearchResult>,
    /// Candidate ids left unvisited when the scan stopped at `limit`;
    /// `0` when the scan exhausted the candidate set.
    pub(crate) unscanned_ids: usize,
}

/// Inverted-similarity threshold bundle for the NOT-similarity scan.
struct NotSimilarityThreshold {
    op: crate::velesql::CompareOp,
    value: f32,
    higher_is_better: bool,
}

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
        let config = self.storage.config.read();
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
                let candidate_vec: std::borrow::Cow<[f32]> = if field == "vector" {
                    std::borrow::Cow::Borrowed(&r.point.vector)
                } else {
                    match self.get_vector_for_field(r.point.id, field) {
                        Ok(Some(v)) => std::borrow::Cow::Owned(v),
                        Ok(None) => return None,
                        Err(e) => {
                            tracing::warn!(
                                point_id = r.point.id, field = field, error = %e,
                                "filter_by_similarity: failed to retrieve named vector field; point skipped"
                            );
                            return None;
                        }
                    }
                };

                // BUG-2 FIX: Recompute similarity using the similarity() vector, not NEAR scores
                let score = self.compute_metric_score(&candidate_vec, query_vec);
                let passes = Self::compare_similarity(score, threshold_f32, op, higher_is_better);

                if passes {
                    // EPIC-044 US-001: Update score to reflect THIS similarity filter's score.
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
    /// With `candidates` (GraphFirst anchor ids), only those ids are
    /// scanned, so the fetch is exact at `limit` within the graph matches;
    /// `None` scans the whole collection.
    pub(crate) fn execute_not_similarity_query_over(
        &self,
        condition: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
        candidates: Option<&std::collections::HashSet<u64>>,
    ) -> Result<Vec<SearchResult>> {
        let (sim_field, sim_vec, sim_op, sim_threshold) =
            self.extract_not_similarity_condition(condition, params)?;

        let all_ids = match candidates {
            Some(ids) => {
                let mut sorted: Vec<u64> = ids.iter().copied().collect();
                sorted.sort_unstable();
                sorted
            }
            None => self.storage.vector_storage.read().ids(),
        };
        let total_count = all_ids.len();
        Self::guard_not_similarity_scan(total_count)?;
        Self::warn_large_scan(total_count, limit);

        let metadata_filter = Self::extract_metadata_filter(condition);
        let filter = metadata_filter
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond)));

        let higher_is_better = self.storage.config.read().metric.higher_is_better();

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let threshold_f32 = sim_threshold as f32;
        let mut results = Vec::new();

        let threshold = NotSimilarityThreshold {
            op: sim_op,
            value: threshold_f32,
            higher_is_better,
        };
        for id in all_ids {
            if let Some(result) = self.eval_not_similarity_candidate(
                id,
                &sim_field,
                &sim_vec,
                &threshold,
                filter.as_ref(),
            ) {
                results.push(result);
                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Evaluates one candidate id for the NOT-similarity scan: returns the
    /// hydrated result when it passes both the inverted similarity threshold
    /// and the metadata filter.
    fn eval_not_similarity_candidate(
        &self,
        id: u64,
        sim_field: &str,
        sim_vec: &[f32],
        threshold: &NotSimilarityThreshold,
        filter: Option<&crate::filter::Filter>,
    ) -> Option<SearchResult> {
        let vector = self.retrieve_vector_for_scan(id, sim_field)?;
        let score = self.compute_metric_score(&vector, sim_vec);
        if Self::compare_similarity(
            score,
            threshold.value,
            threshold.op,
            threshold.higher_is_better,
        ) {
            return None; // excluded by the NOT-similarity threshold
        }
        let payload = self
            .storage
            .payload_storage
            .read()
            .retrieve(id)
            .ok()
            .flatten();
        if is_payload_expired(payload.as_ref(), now_unix_secs()) {
            return None;
        }
        if !Self::passes_metadata_filter(filter, payload.as_ref()) {
            return None;
        }
        Some(SearchResult::new(
            Point {
                id,
                vector,
                payload,
                sparse_vectors: None,
            },
            score,
        ))
    }

    /// Compares a similarity score against a threshold using the given operator.
    ///
    /// Delegates to [`Self::compare_score`] in `where_eval.rs` for the shared
    /// metric-aware comparison logic.
    fn compare_similarity(
        score: f32,
        threshold: f32,
        op: crate::velesql::CompareOp,
        higher_is_better: bool,
    ) -> bool {
        Self::compare_score(score, threshold, op, higher_is_better)
    }

    /// Retrieves a vector for a given field, logging warnings on failure.
    fn retrieve_vector_for_scan(&self, id: u64, field: &str) -> Option<Vec<f32>> {
        match self.get_vector_for_field(id, field) {
            Ok(Some(v)) => Some(v),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(
                    point_id = id, field = %field, error = %e,
                    "failed to retrieve vector field; point skipped"
                );
                None
            }
        }
    }

    /// Checks if a payload passes an optional metadata filter.
    fn passes_metadata_filter(
        filter: Option<&crate::filter::Filter>,
        payload: Option<&serde_json::Value>,
    ) -> bool {
        match filter {
            Some(f) => match payload {
                Some(p) => f.matches(p),
                None => f.matches(&serde_json::Value::Null),
            },
            None => true,
        }
    }

    /// Server-side hard ceiling on the number of vectors a single
    /// `NOT similarity()` query may scan (#901).
    ///
    /// A `NOT similarity()` predicate has no index acceleration and may scan
    /// the entire collection. Beyond this generous ceiling the query is a
    /// likely DoS vector, so it is **rejected** rather than merely warned.
    /// This is a SERVER-controlled constant and cannot be raised by the query.
    const NOT_SIMILARITY_MAX_SCAN: usize = 5_000_000;

    /// Rejects a `NOT similarity()` full scan whose collection size exceeds the
    /// server-side ceiling [`Self::NOT_SIMILARITY_MAX_SCAN`] (#901).
    fn guard_not_similarity_scan(total_count: usize) -> Result<()> {
        if total_count > Self::NOT_SIMILARITY_MAX_SCAN {
            return Err(crate::error::Error::Config(format!(
                "NOT similarity() would scan {total_count} vectors, exceeding the \
                 server scan limit of {}. Add a more selective metadata filter or \
                 use a positive similarity() predicate (index-accelerated).",
                Self::NOT_SIMILARITY_MAX_SCAN
            )));
        }
        Ok(())
    }

    /// Emits a performance warning for large NOT-similarity scans.
    fn warn_large_scan(total_count: usize, limit: usize) {
        if total_count > 10_000 && limit > 1000 {
            tracing::warn!(
                "NOT similarity() query scanning {} documents with LIMIT {}. \
                Consider using a smaller LIMIT for better performance.",
                total_count,
                limit
            );
        }
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
                    crate::error::Error::Query(
                        "NOT clause does not contain a similarity condition".to_string(),
                    )
                })
            }
            crate::velesql::Condition::And(left, right) => {
                // Try left, then right
                self.extract_not_similarity_condition(left, params)
                    .or_else(|_| self.extract_not_similarity_condition(right, params))
            }
            _ => Err(crate::error::Error::Query(
                "Expected NOT similarity() condition".to_string(),
            )),
        }
    }

    /// Fallback method for metadata-only queries without vector search.
    ///
    /// When a secondary index is available for the first `Eq` condition in the
    /// filter, uses the index to narrow the scan to matching IDs only (index-
    /// accelerated scan). Otherwise falls back to a full sequential scan.
    pub(crate) fn execute_scan_query(
        &self,
        filter: &crate::filter::Filter,
        limit: usize,
        cond: Option<&crate::velesql::Condition>,
    ) -> Vec<SearchResult> {
        self.execute_scan_query_tracked(filter, limit, cond).results
    }

    /// Like [`Self::execute_scan_query`], but also reports how many candidate
    /// ids were left unvisited when the scan stopped at `limit` (WO-D2).
    ///
    /// Callers that use `limit` as a hard *scan cap* rather than a user limit
    /// (e.g. [`Self::scan_and_score_by_vector`]) need `unscanned_ids` to
    /// detect — and surface — silent truncation. Behavior and results are
    /// byte-identical to `execute_scan_query`.
    pub(crate) fn execute_scan_query_tracked(
        &self,
        filter: &crate::filter::Filter,
        limit: usize,
        cond: Option<&crate::velesql::Condition>,
    ) -> TrackedScan {
        // Try index-accelerated scan: extract the first Eq condition and use
        // the secondary index to get candidate IDs, then post-filter.
        // The cost model (audit F-4.7, issue #1391) decides whether hydrating
        // the candidate set beats a sequential scan with early exit — the former
        // wins for narrow candidate sets, the latter for broad ones (e.g.
        // IsMobile=1 matching 20% of rows). Both paths return identical results.
        if let Some(candidate_ids) = self.try_index_accelerated_ids(filter) {
            if self.prefer_candidate_scan(candidate_ids.len(), limit, cond) {
                return self.scan_candidate_ids(&candidate_ids, filter, limit);
            }
            // Fall through to sequential scan — cost model prefers full scan
        }

        // Full sequential scan (slow fallback for non-indexed conditions).
        let payload_storage = self.storage.payload_storage.read();
        let vector_storage = self.storage.vector_storage.read();

        let vector_ids = vector_storage.ids();
        let ids: Vec<u64> = if vector_ids.is_empty() {
            payload_storage.ids()
        } else {
            vector_ids
        };
        let total_ids = ids.len();

        // Record rows actually visited as payload-mirror scan debt: once the
        // debt exceeds one full-scan-equivalent, the next metadata query
        // builds the columnar mirror and skips this fallback entirely.
        let mut scanned: u64 = 0;
        let counted_ids = ids.into_iter().inspect(|_| scanned += 1);
        let results = Self::collect_filtered_scan(
            &*payload_storage,
            &*vector_storage,
            counted_ids,
            filter,
            limit,
        );
        self.storage.payload_mirror.add_scan_debt(scanned);
        // `scanned` counts visited ids out of `total_ids`; on 64-bit targets
        // the conversion is lossless (falls back to "all visited" otherwise).
        let visited = usize::try_from(scanned).unwrap_or(total_ids);
        TrackedScan {
            results,
            unscanned_ids: total_ids.saturating_sub(visited),
        }
    }

    /// Shared scan body: iterates `ids`, hydrates each matching point, and
    /// stops once `limit` results are collected. Extracted to keep the full
    /// scan and the candidate-id scan from duplicating the hydration loop.
    fn collect_filtered_scan(
        payload_storage: &dyn PayloadStorage,
        vector_storage: &dyn VectorStorage,
        ids: impl IntoIterator<Item = u64>,
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> Vec<SearchResult> {
        let now_secs = now_unix_secs();
        let mut results = Vec::new();
        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();
            if is_payload_expired(payload.as_ref(), now_secs) {
                continue;
            }
            let matches = match payload {
                Some(ref p) => filter.matches(p),
                None => filter.matches(&serde_json::Value::Null),
            };
            if matches {
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
                    1.0,
                ));
            }
            if results.len() >= limit {
                break;
            }
        }
        results
    }

    /// Tries to extract candidate IDs from secondary indexes for the filter.
    ///
    /// Walks the filter condition tree looking for `Eq` conditions on indexed
    /// fields. Returns the smallest candidate set found, or `None` if no
    /// indexed condition exists.
    fn try_index_accelerated_ids(&self, filter: &crate::filter::Filter) -> Option<Vec<u64>> {
        self.extract_bitmap_ids_from_filter(&filter.condition)
    }

    /// Recursively extracts candidate IDs from a filter condition using indexes.
    fn extract_bitmap_ids_from_filter(
        &self,
        condition: &crate::filter::Condition,
    ) -> Option<Vec<u64>> {
        use crate::filter::Condition;
        match condition {
            Condition::Eq { field, value } => {
                if let Some(jv) = crate::index::JsonValue::from_json(value) {
                    return self.secondary_index_lookup(field, &jv);
                }
                None
            }
            Condition::And { conditions } => {
                // Find the smallest indexed candidate set among AND children.
                let mut best: Option<Vec<u64>> = None;
                for sub in conditions {
                    if let Some(ids) = self.extract_bitmap_ids_from_filter(sub) {
                        best = Some(match best {
                            Some(prev) if prev.len() <= ids.len() => prev,
                            _ => ids,
                        });
                    }
                }
                best
            }
            _ => None,
        }
    }

    /// Scans a pre-filtered set of candidate IDs with the full filter,
    /// tracking how many candidate ids were left unvisited at `limit`.
    fn scan_candidate_ids(
        &self,
        candidate_ids: &[u64],
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> TrackedScan {
        let payload_storage = self.storage.payload_storage.read();
        let vector_storage = self.storage.vector_storage.read();
        let mut scanned: usize = 0;
        let results = Self::collect_filtered_scan(
            &*payload_storage,
            &*vector_storage,
            candidate_ids.iter().copied().inspect(|_| scanned += 1),
            filter,
            limit,
        );
        TrackedScan {
            results,
            unscanned_ids: candidate_ids.len().saturating_sub(scanned),
        }
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
    /// 3. A bounded top-k heap (#901) retains only the `limit` best results in
    ///    `O(limit)` memory, yielding the same top-k as a full sort + truncate.
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
        // #901: bound the scored candidate set with a top-k heap instead of
        // materializing every metadata match and full-sorting. `SCAN_CAP`
        // caps how many *matches* we score (pathological-query guard); the
        // heap caps retained memory to O(limit). Results and ordering are
        // identical to the previous full sort + truncate.
        const SCAN_CAP: usize = 100_000;
        self.scan_and_score_by_vector_capped(metadata_filter, query, limit, SCAN_CAP)
    }

    /// Cap-parameterized body of [`Self::scan_and_score_by_vector`] (WO-D2).
    ///
    /// Split out so tests can prove the truncation warning and result
    /// correctness with a small cap instead of inserting 100k points.
    /// Production callers always go through the public wrapper
    /// (`SCAN_CAP = 100_000`); the truncation behavior itself is unchanged.
    pub(crate) fn scan_and_score_by_vector_capped(
        &self,
        metadata_filter: &crate::filter::Filter,
        query: &[f32],
        limit: usize,
        scan_cap: usize,
    ) -> Vec<SearchResult> {
        let config = self.config();
        let metric = config.metric;
        let higher_is_better = metric.higher_is_better();

        let scan = self.execute_scan_query_tracked(metadata_filter, scan_cap, None);
        if scan.results.len() >= scan_cap && scan.unscanned_ids > 0 {
            // Observability (WO-D2): the #901 pathological-query guard
            // otherwise degrades recall silently once a collection outgrows
            // the cap. Emitted ONCE per query — never per candidate. The
            // remainder is an upper bound: unvisited ids may not all match.
            tracing::warn!(
                collection = %config.name,
                scan_cap,
                matches_scored = scan.results.len(),
                unscanned_points = scan.unscanned_ids,
                "vector full-scan truncated at scan cap: up to `unscanned_points` \
                 remaining points were never scored, so recall may be degraded. \
                 Narrow the metadata filter (ideally on an indexed field) so it \
                 matches fewer points, or split the query."
            );
        }

        let mut topk = super::bounded_top_k::BoundedTopK::new(limit, higher_is_better);
        for mut r in scan.results {
            // Exact distance computation using the stored vector.
            // Clamp is mandatory (SecDev) to guard against NaN from zero-norm vectors.
            r.score = metric.calculate(&r.point.vector, query).clamp(-1.0, 1.0);
            topk.offer(r);
        }

        topk.into_sorted_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #901: a `NOT similarity()` scan within the server ceiling is allowed.
    #[test]
    fn test_not_similarity_guard_allows_within_ceiling() {
        assert!(Collection::guard_not_similarity_scan(Collection::NOT_SIMILARITY_MAX_SCAN).is_ok());
        assert!(Collection::guard_not_similarity_scan(10_000).is_ok());
    }

    /// #901: a `NOT similarity()` scan over the server ceiling is REJECTED
    /// (not merely warned) to block the unbounded-scan DoS vector.
    #[test]
    fn test_not_similarity_guard_rejects_above_ceiling() {
        let err = Collection::guard_not_similarity_scan(Collection::NOT_SIMILARITY_MAX_SCAN + 1)
            .expect_err("scan above ceiling must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("scan limit") || msg.contains("exceeding"),
            "error should explain the scan-limit rejection, got: {msg}"
        );
    }
}
