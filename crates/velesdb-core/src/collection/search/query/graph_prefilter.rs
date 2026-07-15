//! GraphFirst anchor-id prefiltering for SELECT graph predicates.
//!
//! A `MATCH (...)` predicate that is AND-required by the WHERE clause admits
//! only rows whose id is in the predicate's anchor set. Evaluating those
//! predicates FIRST turns hybrid retrieval exhaustive:
//!
//! - **NEAR + MATCH**: selective anchor sets are scored exactly against the
//!   query vector (provably exhaustive); large ones become a `RoaringBitmap`
//!   pushed into filtered HNSW — either way the top-k is found *within* the
//!   graph matches instead of hoping they appear inside a bounded over-fetch
//!   window.
//! - **Unranked + MATCH**: the anchor set IS the candidate set — hydrate it
//!   directly instead of scanning a `MAX_LIMIT` window.
//! - **Sparse + MATCH**: the anchor set feeds the sparse index's per-id
//!   filter, so the fetch is exact at `limit` (see `hybrid_sparse.rs`).
//!
//! Predicates under `OR`/`NOT` are not required and contribute no prefilter;
//! those query shapes keep the post-filter execution. The exact WHERE
//! post-filter always runs afterwards (with the warmed predicate cache, so
//! anchor sets are never evaluated twice).

use super::where_eval::GraphMatchEvalCache;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{Condition, GraphMatchPredicate};
use std::collections::HashSet;

/// How many anchor ids are hydrated per `get` batch on the unranked path.
const ANCHOR_HYDRATION_CHUNK: usize = 1024;

/// Anchor sets up to this size are scored exactly against the query vector
/// (exhaustive); larger sets fall back to the bitmap-filtered HNSW path.
/// Matches the order of magnitude of the GraphFirst scan cap.
const ANCHORED_EXACT_SCORE_MAX: usize = 10_000;

/// Collects graph predicates that are AND-required by `cond`: every result
/// row must satisfy them. Predicates under `Or`/`Not` are skipped (a row may
/// match without them).
pub(super) fn collect_required_graph_predicates(cond: &Condition) -> Vec<&GraphMatchPredicate> {
    let mut out = Vec::new();
    collect_required(cond, &mut out);
    out
}

fn collect_required<'a>(cond: &'a Condition, out: &mut Vec<&'a GraphMatchPredicate>) {
    match cond {
        Condition::GraphMatch(predicate) => out.push(predicate),
        Condition::And(left, right) => {
            collect_required(left, out);
            collect_required(right, out);
        }
        Condition::Group(inner) => collect_required(inner, out),
        _ => {}
    }
}

impl Collection {
    /// Evaluates the AND-required graph predicates of `cond` and returns the
    /// intersection of their anchor sets, warming `cache` so the exact WHERE
    /// post-filter reuses the same sets without re-running the traversals.
    ///
    /// Returns `Ok(None)` when no predicate is AND-required (nothing to
    /// prefilter with). An empty set is meaningful: the query provably has
    /// no results.
    pub(super) fn compute_required_anchor_ids(
        &self,
        cond: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
        cache: &mut GraphMatchEvalCache,
    ) -> Result<Option<HashSet<u64>>> {
        let predicates = collect_required_graph_predicates(cond);
        let Some((first, rest)) = predicates.split_first() else {
            return Ok(None);
        };
        let mut anchors = cache
            .get_or_compute(self, first, params, from_aliases)?
            .clone();
        for predicate in rest {
            if anchors.is_empty() {
                break;
            }
            let ids = cache.get_or_compute(self, predicate, params, from_aliases)?;
            anchors.retain(|id| ids.contains(id));
        }
        Ok(Some(anchors))
    }

    /// Anchored NEAR: returns the top-`limit` vector neighbors *within* the
    /// anchor set.
    ///
    /// Selective anchor sets (≤ [`ANCHORED_EXACT_SCORE_MAX`]) are scored
    /// exactly — every anchor's vector is hydrated and ranked, so retrieval
    /// is provably exhaustive (the HNSW bitmap mechanism only post-filters a
    /// bounded candidate pool and can miss anchors ranked far from the
    /// query). Larger anchor sets use the bitmap path with adaptive retry:
    /// at that density the window bias is negligible and exact scoring
    /// would cost more than it saves.
    pub(crate) fn search_near_with_anchor_ids(
        &self,
        vector: &[f32],
        anchor_ids: &HashSet<u64>,
        filter_condition: Option<&Condition>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if anchor_ids.is_empty() {
            return Ok(Vec::new());
        }
        let metric = self.validate_query_and_read_metric(vector)?;
        let metadata_filter = filter_condition
            .and_then(Self::extract_metadata_filter)
            .map(|c| crate::filter::Filter::new(crate::filter::Condition::from(c)));

        if anchor_ids.len() <= ANCHORED_EXACT_SCORE_MAX {
            return Ok(self.score_anchors_exact(
                vector,
                anchor_ids,
                metadata_filter.as_ref(),
                limit,
                metric.higher_is_better(),
            ));
        }
        self.search_anchors_bitmap(vector, anchor_ids, metadata_filter, limit, metric)
    }

    /// Bitmap-filtered HNSW search over a large anchor set (adaptive retry
    /// in `search_with_quality_and_bitmap` compensates the post-hoc filter).
    fn search_anchors_bitmap(
        &self,
        vector: &[f32],
        anchor_ids: &HashSet<u64>,
        metadata_filter: Option<crate::filter::Filter>,
        limit: usize,
        metric: crate::distance::DistanceMetric,
    ) -> Result<Vec<SearchResult>> {
        let mut bitmap = roaring::RoaringBitmap::new();
        for &id in anchor_ids {
            if let Ok(id32) = u32::try_from(id) {
                bitmap.insert(id32);
            }
        }
        if let Some(meta_bitmap) = metadata_filter
            .as_ref()
            .and_then(|f| self.build_prefilter_bitmap(f))
        {
            bitmap &= meta_bitmap;
        }

        // Residual (non-indexed) metadata conditions are applied after the
        // fetch — oversample for them; pure graph prefilters need no slack.
        let candidates_k = metadata_filter
            .as_ref()
            .map_or(limit, |f| {
                super::super::vector_filter::compute_oversampled_k(limit, f)
            })
            .min(super::MAX_LIMIT);
        let index_results = self.storage.index.search_with_quality_and_bitmap(
            vector,
            candidates_k,
            crate::SearchQuality::default(),
            &bitmap,
        )?;
        let index_results = self.merge_delta(index_results, vector, candidates_k, metric);

        let pass_all =
            crate::filter::Filter::new(crate::filter::Condition::And { conditions: vec![] });
        let filter = metadata_filter.unwrap_or(pass_all);
        Ok(self.filter_and_hydrate(index_results, &filter, limit, metric.higher_is_better()))
    }

    /// Exact anchored scoring: hydrates every anchor, applies the metadata
    /// filter, scores against `query`, and returns the polarity-sorted
    /// top-`limit`. Exhaustive by construction.
    fn score_anchors_exact(
        &self,
        query: &[f32],
        anchor_ids: &HashSet<u64>,
        metadata_filter: Option<&crate::filter::Filter>,
        limit: usize,
        higher_is_better: bool,
    ) -> Vec<SearchResult> {
        let mut ids: Vec<u64> = anchor_ids.iter().copied().collect();
        ids.sort_unstable();

        let mut scored = Vec::new();
        for chunk in ids.chunks(ANCHOR_HYDRATION_CHUNK) {
            for point in self.get(chunk).into_iter().flatten() {
                let payload = point.payload.clone().unwrap_or(serde_json::Value::Null);
                if metadata_filter.is_some_and(|f| !f.matches(&payload)) {
                    continue;
                }
                let score = self.compute_metric_score(&point.vector, query);
                scored.push(SearchResult::new(point, score));
            }
        }
        if higher_is_better {
            scored.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
        } else {
            scored.sort_unstable_by(|a, b| a.score.total_cmp(&b.score));
        }
        scored.truncate(limit);
        scored
    }

    /// Anchored unranked fetch: hydrates the anchor set in deterministic
    /// (ascending id) order, applies the full WHERE per candidate, and stops
    /// at `limit` — replacing the `MAX_LIMIT` scan window with an exact,
    /// exhaustive fetch (the anchors are the only possible matches).
    pub(super) fn fetch_anchor_candidates(
        &self,
        anchor_ids: &HashSet<u64>,
        cond: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
        cache: &mut GraphMatchEvalCache,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let mut ids: Vec<u64> = anchor_ids.iter().copied().collect();
        ids.sort_unstable();

        let mut results = Vec::new();
        for chunk in ids.chunks(ANCHOR_HYDRATION_CHUNK) {
            for point in self.get(chunk).into_iter().flatten() {
                if results.len() >= limit {
                    return Ok(results);
                }
                let passes = self.evaluate_where_condition_for_record(
                    cond,
                    point.id,
                    point.payload.as_ref(),
                    Some(&point.vector),
                    params,
                    from_aliases,
                    cache,
                )?;
                if passes {
                    results.push(SearchResult::new(point, 1.0));
                }
            }
        }
        Ok(results)
    }
}
