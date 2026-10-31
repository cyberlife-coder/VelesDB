//! What a vector search merges besides the index's results: points written
//! but not indexed yet. The async index builder holds the points
//! `upsert_bulk`'s V2 path wrote that the graph has not linked; the deferred
//! indexer buffers writes it has not inserted; the delta buffer holds the
//! writes a rebuild has not seen. Together they are the collection's
//! *unindexed* points.
//!
//! A graph walk cannot reach those points, or reaches a copy older than the
//! one written since, so every search handles them itself, in one place
//! ([`Collection::merge_pending`]). Three rules make that correct whatever
//! the search:
//!
//! - **The unindexed ids are read before the index search**
//!   ([`Collection::snapshot_pending`]). Each of the three sources drops a
//!   point only after indexing it, so a snapshot taken before the walk holds
//!   every point the walk could miss: the point was either indexed before the
//!   walk started, and the walk can reach it, or still held when the snapshot
//!   was taken. Read after the walk, a source could already have dropped a
//!   point the walk had already missed. [`PendingSnapshot`] comes only from
//!   `snapshot_pending`, and [`Collection::search_merging_pending`] takes it
//!   before running the search it is given.
//! - **Storage is the one source of truth for an unindexed point.** Each is
//!   scored on the vector storage holds for it now: a point deleted since is
//!   skipped, one written again since is scored on its newest vector, and an
//!   id several sources hold is scored once. The index's own result for an
//!   unindexed id is dropped before ranking — it may be an older copy — so
//!   only the current one competes, and the index is asked for more when
//!   such ids crowd its `k`, so the next point it ranks is not left out.
//! - **The search's restriction applies before ranking**
//!   ([`PendingRestriction`]): a bitmap of the ids the search admits (a
//!   filter's prefilter, a graph anchor set), and whether a post-filter runs
//!   on the merged list. Merged unrestricted and cut to `k`, unindexed points
//!   outside the restriction could take every place and leave the
//!   post-filter nothing.
//!
//! Scoring reads storage in chunks of [`SCORE_CHUNK`] ids, taking and
//! releasing the `vector_storage` read guard per chunk, so a writer waits at
//! most one chunk's scan rather than the whole one.

use roaring::RoaringBitmap;

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::scored_result::ScoredResult;
use crate::storage::VectorStorage;

/// How many unindexed ids are scored under one `vector_storage` read guard.
const SCORE_CHUNK: usize = 256;

/// What a search admits, applied to the points written but not indexed yet
/// before they are ranked against the index's results.
#[derive(Clone, Copy)]
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

/// The unindexed ids — the async builder's, the deferred indexer's and the
/// delta buffer's, each once, sorted — read before an index search.
pub(crate) struct PendingSnapshot(Vec<u64>);

impl PendingSnapshot {
    /// Whether `id` is unindexed.
    fn holds(&self, id: u64) -> bool {
        self.0.binary_search(&id).is_ok()
    }
}

impl Collection {
    /// Reads every unindexed id. Take it **before** the index search whose
    /// results it is merged with (see the module doc).
    pub(crate) fn snapshot_pending(&self) -> PendingSnapshot {
        let mut ids = self
            .streaming
            .async_index_builder
            .load()
            .as_ref()
            .map(|aib| aib.pending_ids())
            .unwrap_or_default();
        if let Some(deferred) = self.streaming.deferred_indexer.load().as_ref() {
            ids.extend(deferred.ids());
        }
        ids.extend(self.streaming.delta_buffer.ids());
        ids.sort_unstable();
        ids.dedup();
        PendingSnapshot(ids)
    }

    /// Runs `index_search` (given how many results to return) between a
    /// pending snapshot and the merge, so a single-query search cannot take
    /// them in the wrong order, and again for more while unindexed ids crowd
    /// its `k` ([`Self::index_results_past_pending`]).
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
        index_search: impl Fn(usize) -> Result<Vec<ScoredResult>>,
    ) -> Result<Vec<ScoredResult>> {
        let pending = self.snapshot_pending();
        let first = index_search(k)?;
        let results = Self::index_results_past_pending(&pending, k, first, index_search)?;
        Ok(self.merge_pending(results, &pending, query, k, metric, restriction))
    }

    /// `results`, an index search for `k`, or — while the unindexed ids among
    /// them leave fewer than `k` others — `index_search` run again for `k`
    /// plus that many. The merge drops the index's result for an unindexed
    /// id, so without the refetch an id the index still holds, overwritten
    /// since, would take the place of the next point the index ranks rather
    /// than leave it to that point. Each refetch asks for more than the last,
    /// at most `k` plus the snapshot's size, so it stops; none runs unless an
    /// unindexed id is among the results.
    ///
    /// # Errors
    ///
    /// Returns what `index_search` returns.
    pub(crate) fn index_results_past_pending(
        pending: &PendingSnapshot,
        k: usize,
        mut results: Vec<ScoredResult>,
        index_search: impl Fn(usize) -> Result<Vec<ScoredResult>>,
    ) -> Result<Vec<ScoredResult>> {
        let mut fetched = k;
        loop {
            let masked = results.iter().filter(|r| pending.holds(r.id)).count();
            let wanted = k.saturating_add(masked);
            // Fewer than asked for: the index has no more to give.
            if masked == 0 || results.len() < fetched || wanted <= fetched {
                return Ok(results);
            }
            fetched = wanted;
            results = index_search(fetched)?;
        }
    }

    /// Merges `results` with the unindexed points `pending` names that
    /// `restriction` admits, each scored on its current vector, after
    /// dropping the index's result for every unindexed id; keeps the
    /// search's `k` (all of them for a post-filtered search), best first.
    pub(crate) fn merge_pending(
        &self,
        mut results: Vec<ScoredResult>,
        pending: &PendingSnapshot,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
        restriction: PendingRestriction<'_>,
    ) -> Vec<ScoredResult> {
        let ids = &pending.0;
        if ids.is_empty() {
            return results;
        }
        let keep = restriction.keep(k);
        results.retain(|result| !pending.holds(result.id));
        let admitted: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|&id| restriction.admits(id))
            .collect();
        results.extend(self.score_current(query, &admitted));
        metric.top_k_scored_results(&mut results, keep);
        results
    }

    /// Scores each id on the vector storage holds for it now, with the
    /// index's distance and clamp ([`crate::index::hnsw::HnswIndex::score`]);
    /// an id storage no longer holds is skipped.
    fn score_current(&self, query: &[f32], ids: &[u64]) -> Vec<ScoredResult> {
        let index = &self.storage.index;
        let mut scored = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(SCORE_CHUNK) {
            let storage = self.storage.vector_storage.read();
            for &id in chunk {
                let Ok(Some(vector)) = storage.retrieve_ref(id) else {
                    continue;
                };
                if let Ok(vector) = vector.as_slice() {
                    if vector.len() == query.len() {
                        scored.push(ScoredResult::new(id, index.score(query, vector)));
                    }
                }
            }
            drop(storage);
        }
        scored
    }

    /// What storage holds now for each of `ids`, copied out; an id it no
    /// longer holds is left out. What a merge of unindexed points inserts.
    pub(crate) fn current_vectors(&self, ids: &[u64]) -> Vec<(u64, Vec<f32>)> {
        let mut vectors = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(SCORE_CHUNK) {
            let storage = self.storage.vector_storage.read();
            vectors.extend(
                chunk
                    .iter()
                    .filter_map(|&id| storage.retrieve(id).ok().flatten().map(|v| (id, v))),
            );
            drop(storage);
        }
        vectors
    }
}
