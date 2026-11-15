//! The exact `similarity()` scan: every point the WHERE holds for, ranked by
//! the clause's first `similarity()` leaf.
//!
//! A threshold is a filter over the whole collection, not over a window of
//! nearest points. Each candidate is decided by
//! [`Collection::evaluate_where_condition_for_record`] — the one WHERE
//! evaluator, three-valued, each `similarity()` read on its own vector — so
//! `NOT`, `OR` and named vectors need no path of their own. The rows come back
//! most similar first to the first leaf (closest first under a distance), each
//! scored by it; a row that leaf cannot score comes after them, carrying
//! [`crate::velesql::METADATA_MATCH_SCORE`].
//!
//! It is the path for every shape no index can bound: a negated threshold, a
//! threshold under `OR`, and one that is not a "closer than" bound on the
//! point's own vector.

use std::collections::{HashMap, HashSet};

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::Condition;

use super::bounded_top_k::BoundedTopK;

/// The first `similarity()` leaf of a clause, resolved: what every row is
/// ranked and scored by.
struct RankingLeaf {
    field: String,
    query: Vec<f32>,
}

/// Per-scan invariants, hoisted once.
struct ScanCtx<'a> {
    vector_storage: &'a crate::storage::MmapStorage,
    payload_storage: &'a crate::storage::LogPayloadStorage,
    metric: crate::distance::DistanceMetric,
    now_secs: u64,
    ranking: Option<&'a RankingLeaf>,
}

/// The rows a scan keeps: the `limit` best scored ones, and up to `limit`
/// unscored ones that rank after them.
struct ScanRows {
    scored: BoundedTopK,
    unscored: Vec<SearchResult>,
    limit: usize,
}

impl ScanRows {
    fn offer(&mut self, row: SearchResult, scored: bool) {
        if scored {
            self.scored.offer(row);
        } else if self.unscored.len() < self.limit {
            self.unscored.push(row);
        }
    }

    fn into_ranked(self) -> Vec<SearchResult> {
        let mut rows = self.scored.into_sorted_vec();
        rows.extend(self.unscored);
        rows.truncate(self.limit);
        rows
    }
}

impl Collection {
    /// Whether `cond` needs the exact scan rather than an index-bounded
    /// search: a threshold under `NOT` or under `OR` with other predicates,
    /// or — when no `NEAR`, `SPARSE_NEAR` or `NEAR_FUSED` ranks the rows — a
    /// first `similarity()` that is not a "closer than" bound
    /// (`>` / `>=`) on the point's own vector, the only shape whose rows an
    /// HNSW search reaches in ranking order.
    pub(super) fn needs_similarity_scan(
        cond: &Condition,
        leaves: &[super::similarity_filter::SimilarityLeaf],
        ranked_by_a_search: bool,
    ) -> bool {
        use crate::velesql::CompareOp;
        let index_bounded = |(field, _, op, _): &super::similarity_filter::SimilarityLeaf| {
            field == crate::velesql::PRIMARY_VECTOR_FIELD
                && matches!(op, CompareOp::Gt | CompareOp::Gte)
        };
        Self::has_similarity_under_not(cond)
            || Self::has_similarity_in_problematic_or(cond)
            || (!ranked_by_a_search && leaves.first().is_some_and(|leaf| !index_bounded(leaf)))
    }

    /// Server-side hard ceiling on the number of vectors one `similarity()`
    /// scan may visit (#901). A scan has no index acceleration; beyond this
    /// generous ceiling the query is a likely DoS vector, so it is
    /// **rejected** rather than merely warned. This is a SERVER-controlled
    /// constant and cannot be raised by the query.
    const SIMILARITY_SCAN_MAX: usize = 5_000_000;

    /// Rejects a scan whose candidate count exceeds
    /// [`Self::SIMILARITY_SCAN_MAX`] (#901).
    fn guard_similarity_scan(total_count: usize) -> Result<()> {
        if total_count > Self::SIMILARITY_SCAN_MAX {
            return Err(crate::error::Error::Config(format!(
                "this similarity() query would scan {total_count} vectors, exceeding the \
                 server scan limit of {}. Add a more selective metadata filter or \
                 use a positive similarity() threshold on the point's own vector \
                 (index-accelerated).",
                Self::SIMILARITY_SCAN_MAX
            )));
        }
        Ok(())
    }

    /// Emits a performance warning for large scans.
    fn warn_large_scan(total_count: usize, limit: usize) {
        if total_count > 10_000 && limit > 1000 {
            tracing::warn!(
                "similarity() query scanning {} documents with LIMIT {}. \
                Consider using a smaller LIMIT for better performance.",
                total_count,
                limit
            );
        }
    }

    /// The `limit` best points `condition` holds for, ranked and scored by
    /// its first `similarity()` leaf (see the module docs).
    ///
    /// With `candidates` (GraphFirst anchor ids), only those ids are scanned,
    /// so the fetch is exact at `limit` within the graph matches; `None` scans
    /// the whole collection.
    ///
    /// # Errors
    ///
    /// An unresolvable query vector, a scan above
    /// [`Self::SIMILARITY_SCAN_MAX`], or an evaluation error (a nested MATCH).
    // The two storage guards are handed to `MatchStorageGuards` by reference
    // and read throughout the scan, including through `PayloadMemo`. They are
    // acquired once precisely so the scan does not re-take them mid-flight.
    #[expect(clippy::significant_drop_tightening)]
    pub(crate) fn execute_similarity_scan(
        &self,
        condition: &Condition,
        params: &HashMap<String, serde_json::Value>,
        limit: usize,
        candidates: Option<&HashSet<u64>>,
    ) -> Result<Vec<SearchResult>> {
        let ranking = Self::extract_all_similarity_conditions(condition, params)?
            .into_iter()
            .next()
            .map(|(field, query, _, _)| RankingLeaf { field, query });

        let mut all_ids = match candidates {
            Some(ids) => ids.iter().copied().collect(),
            None => self.storage.vector_storage.read().ids(),
        };
        all_ids.sort_unstable();
        let total_count = all_ids.len();
        Self::guard_similarity_scan(total_count)?;
        Self::warn_large_scan(total_count, limit);

        // LOCK ORDER: metric snapshot (config, rank 1) first, then
        // vector_storage (2) before payload_storage (3) — the order a real
        // HTTP deadlock was traced to (tests/tokio_blocking_lock_contention_repro.rs).
        // The guards are then handed to the evaluator so a nested MATCH leaf
        // reuses them instead of re-acquiring the same locks mid-scan.
        let metric = self.storage.config.read().metric;
        let now_secs = now_unix_secs();
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let match_guards = crate::collection::search::query::match_exec::MatchStorageGuards {
            metric,
            vector_guard: &vector_storage,
            payload_guard: &payload_storage,
            payload_memo: crate::collection::search::query::match_exec::PayloadMemo::new(
                &payload_storage,
            ),
            where_filters: crate::collection::search::query::match_exec::WhereFilterMemo::default(),
        };
        let mut graph_cache = super::where_eval::GraphMatchEvalCache::default();
        let scan = ScanCtx {
            vector_storage: &vector_storage,
            payload_storage: &payload_storage,
            metric,
            now_secs,
            ranking: ranking.as_ref(),
        };
        let mut rows = ScanRows {
            scored: BoundedTopK::new(limit, metric.higher_is_better()),
            unscored: Vec::new(),
            limit,
        };
        for id in all_ids {
            if let Some((row, scored)) = self.scan_candidate(
                id,
                condition,
                params,
                &scan,
                &match_guards,
                &mut graph_cache,
            )? {
                rows.offer(row, scored);
            }
        }
        Ok(rows.into_ranked())
    }

    /// Evaluates one candidate against the full condition; when it holds,
    /// the hydrated row and whether the ranking leaf scored it.
    fn scan_candidate(
        &self,
        id: u64,
        condition: &Condition,
        params: &HashMap<String, serde_json::Value>,
        scan: &ScanCtx<'_>,
        guards: &crate::collection::search::query::match_exec::MatchStorageGuards<'_>,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Option<(SearchResult, bool)>> {
        // The SELECT universe of a vector collection: points with a vector.
        let Ok(Some(vector)) = scan.vector_storage.retrieve(id) else {
            return Ok(None);
        };
        let payload = scan.payload_storage.retrieve(id).ok().flatten();
        if is_payload_expired(payload.as_ref(), scan.now_secs) {
            return Ok(None);
        }
        if !self.evaluate_where_condition_for_record(
            condition,
            id,
            payload.as_ref(),
            Some(&vector),
            params,
            &[],
            graph_cache,
            Some(guards),
        )? {
            return Ok(None);
        }
        let score = scan
            .ranking
            .and_then(|leaf| ranking_score(leaf, &vector, payload.as_ref(), scan.metric));
        let row = SearchResult::new(
            Point {
                id,
                vector,
                payload,
                sparse_vectors: None,
            },
            score.unwrap_or(crate::velesql::METADATA_MATCH_SCORE),
        );
        Ok(Some((row, score.is_some())))
    }
}

/// The score of a row against the ranking leaf: `None` when the row has no
/// vector of the query's dimension under the leaf's field.
fn ranking_score(
    leaf: &RankingLeaf,
    own: &[f32],
    payload: Option<&serde_json::Value>,
    metric: crate::distance::DistanceMetric,
) -> Option<f32> {
    let named;
    let vector: &[f32] = if leaf.field == crate::velesql::PRIMARY_VECTOR_FIELD {
        own
    } else {
        named = crate::velesql::payload_vector(payload?, &leaf.field).ok()??;
        &named
    };
    (vector.len() == leaf.query.len() && !vector.is_empty())
        .then(|| metric.calculate(vector, &leaf.query))
}

#[cfg(test)]
#[path = "similarity_scan_tests.rs"]
mod tests;
