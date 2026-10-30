//! What a vector search merges besides the index's results: points written
//! but not indexed yet. The async index builder holds the points
//! `upsert_bulk`'s V2 path wrote that the graph has not linked; the deferred
//! indexer buffers writes it has not inserted; the delta buffer holds the
//! writes a rebuild has not seen.
//!
//! A graph walk cannot reach those points, so every search scores them
//! itself and merges them with the index's results, all three in one place
//! ([`Collection::merge_pending`]). Two rules make that merge correct
//! whatever the search:
//!
//! - **The async builder's pending ids are read before the index search.**
//!   A flush links a point and only then drops it from the builder's queue,
//!   so a snapshot taken before the walk holds every point the walk could
//!   miss: the point was either linked before the walk started, and the walk
//!   can reach it, or still queued when the snapshot was taken. Read after
//!   the walk, the queue could already have lost a point the walk had
//!   already missed. [`PendingSnapshot`] comes only from
//!   [`Collection::snapshot_pending`], and
//!   [`Collection::search_merging_pending`] takes it before running the
//!   search it is given.
//! - **The search's restriction applies to every source's points before
//!   they compete for its `k`** ([`PendingRestriction`]): a bitmap of the
//!   ids the search admits (a filter's prefilter, a graph anchor set), and
//!   whether a post-filter runs on the merged list. Merged unrestricted and
//!   cut to `k`, unindexed points outside the restriction could take every
//!   place and leave the post-filter nothing.
//!
//! The async builder's points are scored on what the collection holds for
//! each now: a placed id on the arena slot its mapping names, a buffered id
//! on the vector storage. A point deleted since is skipped, one upserted
//! since is scored on its new vector, and an id queued twice is scored once.
//! The deferred indexer and the delta buffer score the vectors they hold,
//! the deferred indexer skipping the ids deleted since.

use roaring::RoaringBitmap;

use crate::collection::streaming::PendingIds;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::scored_result::ScoredResult;
use crate::storage::VectorStorage;

/// What a search admits, applied to the points written but not indexed yet
/// before they are ranked against the index's results.
#[derive(Clone, Copy, Default)]
pub(crate) struct PendingRestriction<'a> {
    /// Only ids this bitmap holds, admitted as the index's bitmap searches
    /// admit them: an id past `u32::MAX`, which a bitmap cannot hold, passes.
    bitmap: Option<&'a RoaringBitmap>,
    /// The caller filters the merged list and keeps its own `k` from what
    /// passes: every admitted pending point is kept, so none takes the place
    /// of an index result the filter would have kept.
    post_filtered: bool,
}

impl<'a> PendingRestriction<'a> {
    /// Every pending point competes for the search's `k`.
    pub(crate) const NONE: Self = Self {
        bitmap: None,
        post_filtered: false,
    };

    /// Only the ids `bitmap` holds.
    pub(crate) const fn within(bitmap: &'a RoaringBitmap) -> Self {
        Self {
            bitmap: Some(bitmap),
            post_filtered: false,
        }
    }

    /// A post-filter runs on the merged list when `post_filtered` is set.
    #[must_use]
    pub(crate) const fn post_filtered_if(self, post_filtered: bool) -> Self {
        Self {
            bitmap: self.bitmap,
            post_filtered,
        }
    }

    /// A post-filter runs on the merged list, besides `self`.
    #[must_use]
    pub(crate) const fn and_post_filtered(self) -> Self {
        self.post_filtered_if(true)
    }

    /// A post-filter runs on the merged list.
    pub(crate) const fn post_filtered() -> Self {
        Self::NONE.and_post_filtered()
    }

    fn admits(&self, id: u64) -> bool {
        self.bitmap
            .is_none_or(|bitmap| u32::try_from(id).map_or(true, |id32| bitmap.contains(id32)))
    }

    /// How many merged results the search keeps.
    const fn keep(&self, k: usize) -> usize {
        if self.post_filtered {
            usize::MAX
        } else {
            k
        }
    }
}

/// The async index builder's pending ids, read before an index search.
pub(crate) struct PendingSnapshot(Option<PendingIds>);

impl Collection {
    /// Reads the async index builder's pending ids. Take it **before** the
    /// index search whose results it is merged with (see the module doc).
    pub(crate) fn snapshot_pending(&self) -> PendingSnapshot {
        let slot = self.streaming.async_index_builder.load();
        PendingSnapshot(
            slot.as_ref()
                .map(|aib| aib.pending_ids())
                .filter(|ids| !ids.is_empty()),
        )
    }

    /// Runs `index_search` between a pending snapshot and the merge, so a
    /// single-query search cannot take them in the wrong order.
    ///
    /// # Errors
    ///
    /// Returns what `index_search` returns.
    pub(crate) fn search_merging_pending(
        &self,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
        restriction: PendingRestriction<'_>,
        index_search: impl FnOnce() -> Result<Vec<ScoredResult>>,
    ) -> Result<Vec<ScoredResult>> {
        let pending = self.snapshot_pending();
        let results = index_search()?;
        Ok(self.merge_pending(results, &pending, query, k, metric, restriction))
    }

    /// Merges `results` with every point written but not indexed yet that
    /// `restriction` admits — the async index builder's (from `pending`),
    /// the deferred indexer's and the delta buffer's — keeping the search's
    /// `k` (all of them for a post-filtered search).
    ///
    /// The three are one candidate list, merged once: an id more than one
    /// holds is taken from the async builder's current read first, then the
    /// deferred indexer, then the delta buffer, and any of them replaces the
    /// index's result for it.
    pub(crate) fn merge_pending(
        &self,
        results: Vec<ScoredResult>,
        pending: &PendingSnapshot,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
        restriction: PendingRestriction<'_>,
    ) -> Vec<ScoredResult> {
        let keep = restriction.keep(k);
        let mut candidates = match &pending.0 {
            Some(ids) => self.score_pending(ids, query, metric, restriction, keep),
            None => Vec::new(),
        };
        let admits = |id: u64| restriction.admits(id);
        let deferred_slot = self.streaming.deferred_indexer.load();
        if let Some(deferred) = deferred_slot.as_ref().filter(|di| di.is_searchable()) {
            extend_new_ids(
                &mut candidates,
                deferred.search_admitted(query, keep, metric, admits),
            );
        }
        if self.streaming.delta_buffer.is_searchable() {
            extend_new_ids(
                &mut candidates,
                self.streaming
                    .delta_buffer
                    .search_admitted(query, keep, metric, admits),
            );
        }
        crate::collection::streaming::merge_pending_scored(results, candidates, keep, metric)
    }

    /// Scores the admitted pending ids on what the collection holds for each
    /// now, best `keep` first. A placed id is read from its arena slot, a
    /// buffered one from the vector storage, copied out so the storage lock
    /// is not held while scoring; an id with neither is gone and skipped.
    fn score_pending(
        &self,
        ids: &PendingIds,
        query: &[f32],
        metric: DistanceMetric,
        restriction: PendingRestriction<'_>,
        keep: usize,
    ) -> Vec<(u64, f32)> {
        let index = &self.storage.index;
        let placed: Vec<u64> = ids
            .placed
            .iter()
            .copied()
            .filter(|&id| restriction.admits(id))
            .collect();
        let mut scored = index.score_ids(query, &placed, keep);

        let buffered: Vec<(u64, Vec<f32>)> = {
            let storage = self.storage.vector_storage.read();
            ids.buffered
                .iter()
                .copied()
                .filter(|&id| restriction.admits(id))
                .filter_map(|id| storage.retrieve(id).ok().flatten().map(|v| (id, v)))
                .collect()
        };
        scored.extend(
            buffered
                .iter()
                .filter(|(_, v)| v.len() == query.len())
                .map(|(id, v)| ScoredResult::new(*id, index.compute_distance(query, v))),
        );

        for result in &mut scored {
            result.score = metric.clamp_score(result.score);
        }
        metric.top_k_scored_results(&mut scored, keep);
        scored.into_iter().map(Into::into).collect()
    }
}

/// Appends the `(id, score)` pairs of `more` whose id `candidates` lacks.
fn extend_new_ids(candidates: &mut Vec<(u64, f32)>, more: Vec<(u64, f32)>) {
    if candidates.is_empty() {
        *candidates = more;
        return;
    }
    let seen: rustc_hash::FxHashSet<u64> = candidates.iter().map(|&(id, _)| id).collect();
    candidates.extend(more.into_iter().filter(|(id, _)| !seen.contains(id)));
}
