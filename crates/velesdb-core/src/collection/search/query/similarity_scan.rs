//! The exact `similarity()` scan: every point the WHERE holds for, ranked by
//! the clause's first `similarity()` leaf.
//!
//! A threshold is a filter over the whole collection, not over a window of
//! nearest points. Each candidate is decided by
//! [`Collection::evaluate_where_condition_for_record`] — the one WHERE
//! evaluator, three-valued, each `similarity()` read on its own vector — so
//! `NOT`, `OR` and named vectors need no path of their own. The rows come back
//! most similar first to the first leaf (closest first under a distance), each
//! scored by it and tagged with that score as its `vector_score` component; a
//! row that leaf cannot score comes after them, carrying
//! [`crate::velesql::METADATA_MATCH_SCORE`] and no component breakdown.
//!
//! It is the path for every shape no index can bound: a negated threshold, a
//! threshold under `OR`, and one that is not a "closer than" bound on the
//! point's own vector.
//!
//! It reads every row a SELECT reads ([`RowUniverse`]): a row without a
//! vector of its own — a payload-only graph node, a metadata row — leaves a
//! `similarity(vector, …)` leaf UNKNOWN, while its named vectors and its
//! other predicates still decide it, as they do for `COUNT(*)`.

use std::collections::{HashMap, HashSet};

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::Condition;

use super::bounded_top_k::BoundedTopK;
use super::row_universe::RowUniverse;

/// The first `similarity()` leaf of a clause, resolved: what every row is
/// ranked and scored by.
pub(super) struct RankingLeaf {
    pub(super) field: String,
    pub(super) query: Vec<f32>,
}

/// What a scanned row is scored by.
#[derive(Clone, Copy)]
pub(super) enum RowScore<'a> {
    /// A `similarity()` leaf: a row it cannot score carries
    /// [`crate::velesql::METADATA_MATCH_SCORE`] and no component.
    Leaf(&'a RankingLeaf),
    /// A `NEAR`'s query vector, on the row's own vector, as
    /// [`Collection::scan_and_score_by_vector`] scores it: a NaN takes the
    /// metric's worst score.
    Near(&'a [f32]),
    /// Nothing: every row carries [`crate::velesql::METADATA_MATCH_SCORE`].
    Unscored,
}

/// Per-scan invariants, hoisted once.
pub(super) struct ScanCtx<'a> {
    pub(super) vector_storage: &'a crate::storage::MmapStorage,
    pub(super) payload_storage: &'a crate::storage::LogPayloadStorage,
    pub(super) universe: RowUniverse,
    pub(super) metric: crate::distance::DistanceMetric,
    pub(super) now_secs: u64,
    pub(super) score: RowScore<'a>,
    /// The FROM side's names a graph predicate's anchor resolves against.
    pub(super) from_aliases: &'a [String],
}

/// The rows a scan keeps: the `limit` best scored ones, and up to `limit`
/// unscored ones that rank after them.
struct ScanRows {
    scored: BoundedTopK,
    unscored: Vec<SearchResult>,
    limit: usize,
}

impl ScanRows {
    /// No row yet: the `limit` best scored rows, ranked in the metric's
    /// direction, then up to `limit` unscored ones.
    fn new(limit: usize, higher_is_better: bool) -> Self {
        Self {
            scored: BoundedTopK::new(limit, higher_is_better),
            unscored: Vec::new(),
            limit,
        }
    }

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
    pub(super) const SIMILARITY_SCAN_MAX: usize = 5_000_000;

    /// Rejects a scan whose candidate count exceeds the server ceiling (#901).
    fn guard_similarity_scan(total_count: usize, ceiling: usize) -> Result<()> {
        if total_count > ceiling {
            return Err(crate::error::Error::Config(format!(
                "this similarity() query would scan {total_count} vectors, exceeding the \
                 server scan limit of {ceiling}. Add a more selective metadata filter or \
                 use a positive similarity() threshold on the point's own vector \
                 (index-accelerated)."
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

    /// The leaf a scan ranks and scores by: the first `similarity()`.
    pub(super) fn ranking_leaf(
        condition: &Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Option<RankingLeaf>> {
        Ok(Self::extract_all_similarity_conditions(condition, params)?
            .into_iter()
            .next()
            .map(|(field, query, _, _)| RankingLeaf { field, query }))
    }

    /// The ids a scan visits, in id order: the `candidates` when given, else
    /// every row ([`Self::row_ids`]). Both executors use
    /// [`Self::SIMILARITY_SCAN_MAX`] as their candidate ceiling.
    pub(super) fn similarity_scan_ids<const MAX: usize>(
        &self,
        candidates: Option<&HashSet<u64>>,
        limit: usize,
    ) -> Result<Vec<u64>> {
        let mut ids: Vec<u64> = match candidates {
            Some(ids) => ids.iter().copied().collect(),
            None => self.row_ids(),
        };
        Self::guard_similarity_scan(ids.len(), MAX)?;
        Self::warn_large_scan(ids.len(), limit);
        ids.sort_unstable();
        Ok(ids)
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
        let ranking = Self::ranking_leaf(condition, params)?;
        let all_ids =
            self.similarity_scan_ids::<{ Self::SIMILARITY_SCAN_MAX }>(candidates, limit)?;

        // LOCK ORDER: metric snapshot (config, rank 1) first, then
        // vector_storage (2) before payload_storage (3) — the order a real
        // HTTP deadlock was traced to (tests/tokio_blocking_lock_contention_repro.rs).
        // The guards are then handed to the evaluator so a nested MATCH leaf
        // reuses them instead of re-acquiring the same locks mid-scan.
        let (metric, universe) = {
            let config = self.storage.config.read();
            (config.metric, RowUniverse::of(&config))
        };
        let now_secs = now_unix_secs();
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let match_guards = super::match_exec::MatchStorageGuards::single(
            metric,
            universe,
            &vector_storage,
            &payload_storage,
        );
        let mut graph_cache = super::where_eval::GraphMatchEvalCache::default();
        let scan = ScanCtx {
            vector_storage: &vector_storage,
            payload_storage: &payload_storage,
            universe,
            metric,
            now_secs,
            score: ranking.as_ref().map_or(RowScore::Unscored, RowScore::Leaf),
            from_aliases: &[],
        };
        let mut rows = ScanRows::new(limit, metric.higher_is_better());
        for id in all_ids {
            if let Some((row, scored)) = self.scan_candidate(
                id,
                Some(condition),
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

    /// Evaluates one candidate against the full condition (none: every row
    /// holds); when it holds, the hydrated row and whether its score scored
    /// it.
    pub(super) fn scan_candidate(
        &self,
        id: u64,
        condition: Option<&Condition>,
        params: &HashMap<String, serde_json::Value>,
        scan: &ScanCtx<'_>,
        guards: &super::match_exec::MatchStorageGuards<'_>,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Option<(SearchResult, bool)>> {
        let vector = scan.vector_storage.retrieve(id).ok().flatten();
        let payload = scan.payload_storage.retrieve(id).ok().flatten();
        // A candidate id (a GraphFirst anchor) may name no row.
        if !scan.universe.admits(vector.is_some(), payload.is_some())
            || is_payload_expired(payload.as_ref(), scan.now_secs)
        {
            return Ok(None);
        }
        let holds = match condition {
            Some(condition) => self.evaluate_where_condition_for_record(
                condition,
                id,
                payload.as_ref(),
                vector.as_deref(),
                params,
                scan.from_aliases,
                graph_cache,
                Some(guards),
            )?,
            None => true,
        };
        if !holds {
            return Ok(None);
        }
        let score = row_score(scan.score, vector.as_deref(), payload.as_ref(), scan.metric);
        let mut row = SearchResult::new(
            Point {
                id,
                vector: vector.unwrap_or_default(),
                payload,
                sparse_vectors: None,
            },
            score.unwrap_or(crate::velesql::METADATA_MATCH_SCORE),
        );
        // A row the ranking leaf scored carries that similarity as its
        // vector component; an unscored one carries no breakdown.
        if score.is_some() {
            crate::collection::search::tag_vector_component_scores(std::slice::from_mut(&mut row));
        }
        Ok(Some((row, score.is_some())))
    }
}

/// The score `score` gives a row: `None` when it gives none.
fn row_score(
    score: RowScore<'_>,
    own: Option<&[f32]>,
    payload: Option<&serde_json::Value>,
    metric: crate::distance::DistanceMetric,
) -> Option<f32> {
    match score {
        RowScore::Leaf(leaf) => ranking_score(leaf, own, payload, metric),
        RowScore::Near(query) => {
            let raw = metric.checked_calculate(own?, query)?;
            Some(if raw.is_nan() {
                worst_score(metric)
            } else {
                raw
            })
        }
        RowScore::Unscored => None,
    }
}

/// The metric's worst score, which a NaN takes so it never outranks a row.
fn worst_score(metric: crate::distance::DistanceMetric) -> f32 {
    if metric.higher_is_better() {
        f32::NEG_INFINITY
    } else {
        f32::INFINITY
    }
}

/// The score of a row against the ranking leaf: `None` when the row has no
/// vector of the query's dimension under the leaf's field — its own
/// (`own`, absent from a row that stores none) or a payload's named one.
fn ranking_score(
    leaf: &RankingLeaf,
    own: Option<&[f32]>,
    payload: Option<&serde_json::Value>,
    metric: crate::distance::DistanceMetric,
) -> Option<f32> {
    let named;
    let vector: &[f32] = if leaf.field == crate::velesql::PRIMARY_VECTOR_FIELD {
        own?
    } else {
        named = crate::velesql::payload_vector(payload?, &leaf.field).ok()??;
        &named
    };
    metric.checked_calculate(vector, &leaf.query)
}

#[cfg(test)]
#[path = "similarity_scan_tests.rs"]
mod tests;
