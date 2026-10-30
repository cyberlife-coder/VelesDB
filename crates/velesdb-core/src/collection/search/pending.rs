//! What a vector search merges from the async index builder: the points
//! `upsert_bulk`'s V2 path wrote that the graph has not linked yet.
//!
//! A graph walk cannot reach an unlinked point, so every search scores the
//! builder's pending points itself and merges them with the index's results
//! ([`Collection::merge_delta`]). Two rules make that merge correct whatever
//! the search:
//!
//! - **The pending ids are read before the index search.** A flush links a
//!   point and only then drops it from the builder's queue, so a snapshot
//!   taken before the walk holds every point the walk could miss: the point
//!   was either linked before the walk started, and the walk can reach it,
//!   or still queued when the snapshot was taken. Read after the walk, the
//!   queue could already have lost a point the walk had already missed.
//!   [`PendingSnapshot`] comes only from [`Collection::snapshot_pending`], and
//!   [`Collection::search_merging_pending`] takes it before running the
//!   search it is given.
//! - **The search's restriction applies to the pending points before they
//!   compete for its `k`** ([`PendingRestriction`]): a bitmap of the ids the
//!   search admits (a filter's prefilter, a graph anchor set), and whether a
//!   post-filter runs on the merged list. Merged unrestricted and cut to
//!   `k`, pending points outside the restriction could take every place and
//!   leave the post-filter nothing.
//!
//! Each pending point is scored on what the collection holds for it now: a
//! placed id on the arena slot its mapping names, a buffered id on the
//! vector storage. A point deleted since is skipped, one upserted since is
//! scored on its new vector, and an id queued twice is scored once.

use roaring::RoaringBitmap;

use crate::collection::streaming::PendingIds;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::scored_result::ScoredResult;
use crate::storage::VectorStorage;

/// What a search admits, applied to the builder's pending points before they
/// are ranked against the index's results.
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
        Ok(self.merge_delta(results, &pending, query, k, metric, restriction))
    }

    /// Merges `results` with the pending points `pending` names that
    /// `restriction` admits, scored now, keeping the search's `k` (all of
    /// them for a post-filtered search).
    pub(super) fn merge_pending(
        &self,
        results: Vec<ScoredResult>,
        pending: &PendingSnapshot,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
        restriction: PendingRestriction<'_>,
    ) -> Vec<ScoredResult> {
        let Some(ids) = &pending.0 else {
            return results;
        };
        let keep = restriction.keep(k);
        let scored = self.score_pending(ids, query, metric, restriction, keep);
        crate::collection::streaming::merge_pending_scored(results, scored, keep, metric)
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
