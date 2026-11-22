//! Similarity filtering, NOT-similarity queries, and scan fallbacks.
//!
//! Extracted from query/mod.rs for complexity reduction (EPIC-044).

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

use super::row_universe::{RowScope, RowUniverse};

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

/// The storage guards a scan reads, with the rows they hold.
#[derive(Clone, Copy)]
struct ScanStores<'a> {
    vector_storage: &'a dyn VectorStorage,
    payload_storage: &'a dyn PayloadStorage,
    universe: RowUniverse,
}

/// One `similarity(field, vector) <op> threshold` leaf, its query vector
/// resolved: `(field, query vector, operator, threshold)`.
pub(crate) type SimilarityLeaf = (String, Vec<f32>, crate::velesql::CompareOp, f64);

impl Collection {
    /// Keeps the candidates every `similarity()` leaf of `leaves` holds for
    /// (an AND cascade), each paired with its score against the FIRST leaf —
    /// the score a row carries when no NEAR ranks it.
    ///
    /// Each leaf is recomputed against its own query vector and field, never
    /// read from the candidates' NEAR scores (BUG-2): `similarity(vector, $v)`
    /// reads the point's own vector, `similarity(field, $v)` the payload's
    /// named vector. A candidate a leaf cannot score — no such named vector,
    /// another dimension — is excluded rather than given a fabricated score
    /// (a fake 0.0 used to read as a perfect match for distance metrics).
    ///
    /// The comparison is metric-aware ([`crate::velesql::similarity_holds`]):
    /// under a distance metric `similarity() > t` keeps the candidates
    /// closer than `t`.
    pub(crate) fn filter_by_similarity(
        &self,
        candidates: Vec<SearchResult>,
        leaves: &[SimilarityLeaf],
        limit: usize,
    ) -> Vec<(SearchResult, f32)> {
        let higher_is_better = self.storage.config.read().metric.higher_is_better();
        candidates
            .into_iter()
            .filter_map(|r| {
                let mut scores = leaves
                    .iter()
                    .map(|leaf| self.score_if_holds(&r, leaf, higher_is_better));
                let first = scores.next()??;
                if scores.any(|score| score.is_none()) {
                    return None;
                }
                Some((r, first))
            })
            .take(limit)
            .collect()
    }

    /// The score of candidate `r` against `leaf` when the leaf holds for it;
    /// `None` when it does not, or cannot be scored.
    fn score_if_holds(
        &self,
        r: &SearchResult,
        (field, query_vec, op, threshold): &SimilarityLeaf,
        higher_is_better: bool,
    ) -> Option<f32> {
        let candidate_vec: std::borrow::Cow<[f32]> = if field
            == crate::velesql::PRIMARY_VECTOR_FIELD
        {
            std::borrow::Cow::Borrowed(&r.point.vector)
        } else {
            match self.get_vector_for_field(r.point.id, field) {
                Ok(vector) => std::borrow::Cow::Owned(vector?),
                Err(e) => {
                    tracing::warn!(
                        point_id = r.point.id, field = field, error = %e,
                        "filter_by_similarity: failed to retrieve named vector field; point skipped"
                    );
                    return None;
                }
            }
        };
        let score = self.compute_metric_score(&candidate_vec, query_vec)?;
        crate::velesql::similarity_holds(*op, score, *threshold, higher_is_better).then_some(score)
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
        self.scan_rows_tracked(RowScope::All, filter, limit, cond)
    }

    /// The tracked scan over the rows `scope` reads: every SELECT row, or —
    /// for a vector ranking — only those storing a vector.
    // vector_storage(2) before payload_storage(3), and held together for the
    // whole scan. This pair's order is not incidental: it was reversed here
    // and caused the deadlock reproduced by
    // tests/tokio_blocking_lock_contention_repro.rs. Tightening either guard
    // reopens that ordering question, so both are held deliberately.
    #[expect(clippy::significant_drop_tightening)]
    fn scan_rows_tracked(
        &self,
        scope: RowScope,
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
                return self.scan_candidate_ids(scope, &candidate_ids, filter, limit);
            }
            // Fall through to sequential scan — cost model prefers full scan
        }

        // Full sequential scan (slow fallback for non-indexed conditions).
        // LOCK ORDER: config(1), then vector_storage(2) before
        // payload_storage(3) — the pair was reversed here. See
        // tests/tokio_blocking_lock_contention_repro.rs.
        let universe = scope.universe(self.row_universe());
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();

        let ids = universe.ids(&vector_storage, &payload_storage);
        let total_ids = ids.len();

        // Record rows actually visited as payload-mirror scan debt: once the
        // debt exceeds one full-scan-equivalent, the next metadata query
        // builds the columnar mirror and skips this fallback entirely.
        let mut scanned: u64 = 0;
        let counted_ids = ids.into_iter().inspect(|_| scanned += 1);
        let results = Self::collect_filtered_scan(
            &ScanStores {
                vector_storage: &*vector_storage,
                payload_storage: &*payload_storage,
                universe,
            },
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
        stores: &ScanStores<'_>,
        ids: impl IntoIterator<Item = u64>,
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> Vec<SearchResult> {
        let now_secs = now_unix_secs();
        let ScanStores {
            vector_storage,
            payload_storage,
            universe,
        } = *stores;
        let mut results = Vec::new();
        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();
            if is_payload_expired(payload.as_ref(), now_secs) {
                continue;
            }
            let matches = filter.matches_point(id, payload.as_ref());
            if matches {
                let vector = vector_storage.retrieve(id).ok().flatten();
                // Candidate lists can name ids outside the row universe
                // (`id IN (...)` literals, bitmap supersets): a payload-only
                // id of a vector collection (the residue of a torn delete)
                // is no row, nor is an id naming nothing at all.
                if !universe.admits(vector.is_some(), payload.is_some()) {
                    continue;
                }
                results.push(SearchResult::new(
                    Point {
                        id,
                        vector: vector.unwrap_or_default(),
                        payload,
                        sparse_vectors: None,
                    },
                    crate::velesql::METADATA_MATCH_SCORE,
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
    pub(super) fn try_index_accelerated_ids(
        &self,
        filter: &crate::filter::Filter,
    ) -> Option<Vec<u64>> {
        self.extract_bitmap_ids_from_filter(&filter.condition)
    }

    /// Recursively extracts candidate IDs from a filter condition using indexes.
    fn extract_bitmap_ids_from_filter(
        &self,
        condition: &crate::filter::Condition,
    ) -> Option<Vec<u64>> {
        use crate::filter::Condition;
        match condition {
            Condition::And { conditions } => conditions
                .iter()
                .filter_map(|sub| self.extract_bitmap_ids_from_filter(sub))
                .min_by_key(Vec::len),
            // `id = n` / `id IN (...)` name their points: no index needed.
            leaf => leaf.point_ids().or_else(|| self.indexed_eq_ids(leaf)),
        }
    }

    /// The ids a secondary index holds for an `Eq` leaf on an indexed field.
    fn indexed_eq_ids(&self, condition: &crate::filter::Condition) -> Option<Vec<u64>> {
        let crate::filter::Condition::Eq { field, value } = condition else {
            return None;
        };
        let key = crate::index::JsonValue::from_json(value)?;
        self.secondary_index_lookup(field, &key)
    }

    /// Scans a pre-filtered set of candidate IDs with the full filter,
    /// tracking how many candidate ids were left unvisited at `limit`.
    // Same documented pair and the same post-mortem as
    // `execute_scan_query_tracked`: vector_storage(2) before
    // payload_storage(3), both held across the scan.
    #[expect(clippy::significant_drop_tightening)]
    fn scan_candidate_ids(
        &self,
        scope: RowScope,
        candidate_ids: &[u64],
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> TrackedScan {
        // LOCK ORDER: config(1), then vector_storage(2) before
        // payload_storage(3) — the pair was reversed here. See
        // tests/tokio_blocking_lock_contention_repro.rs.
        let universe = scope.universe(self.row_universe());
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let mut scanned: usize = 0;
        let results = Self::collect_filtered_scan(
            &ScanStores {
                vector_storage: &*vector_storage,
                payload_storage: &*payload_storage,
                universe,
            },
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
    /// Only rows storing a vector are scanned ([`RowScope::Embedded`]): a
    /// row without one — a payload-only graph node — cannot answer a NEAR,
    /// as the HNSW path it stands in for never returns one.
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
    /// A NaN score (corrupt or NaN-bearing stored vector) is mapped to the
    /// metric's worst key so it can never outrank a real result. Scores are
    /// otherwise reported verbatim: Euclidean/Hamming distances and raw dot
    /// products legitimately exceed 1, and cosine is already clamped inside
    /// `DistanceMetric::calculate`.
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

        let scan = self.scan_rows_tracked(RowScope::Embedded, metadata_filter, scan_cap, None);
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

        // Regression (#2101): this used to clamp every metric into [-1, 1],
        // which collapsed all Euclidean/Hamming distances > 1 and all raw dot
        // products > 1 into an artificial tie at 1.0 (arbitrary top-k), while
        // not even removing NaN — clamp() propagates it. Only NaN needs
        // guarding, with the metric-appropriate worst key (same convention as
        // `similarity_scores_for_field`).
        let worst = if higher_is_better {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        };
        let mut topk = super::bounded_top_k::BoundedTopK::new(limit, higher_is_better);
        for mut r in scan.results {
            // Exact distance computation using the stored vector. The scope
            // admits only stored vectors, of the collection's dimension, as
            // `query` is checked to be; a vector that still cannot be
            // compared is no candidate rather than a kernel panic.
            let Some(raw) = metric.checked_calculate(&r.point.vector, query) else {
                continue;
            };
            r.score = if raw.is_nan() { worst } else { raw };
            topk.offer(r);
        }

        // The rescored rows answer a NEAR: their score is the vector
        // component, tagged as the HNSW path tags it.
        let mut results = topk.into_sorted_vec();
        crate::collection::search::tag_vector_component_scores(&mut results);
        results
    }
}

#[cfg(test)]
#[path = "similarity_filter_tests.rs"]
mod tests;
