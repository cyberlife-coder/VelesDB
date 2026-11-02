//! What a vector search merges besides the index's results: points written
//! but not indexed yet. The async index builder holds the points
//! `upsert_bulk`'s V2 path wrote that the graph has not linked; the deferred
//! indexer buffers writes it has not inserted. Together they are the
//! collection's *unindexed* points.
//!
//! A graph walk cannot reach those points, or reaches a copy older than the
//! one written since, so every search handles them itself, in one place
//! ([`Collection::merge_pending`]). Three rules make that correct whatever
//! the search:
//!
//! - **The unindexed ids are read before the index search**
//!   ([`Collection::snapshot_pending`]). Each of the two sources drops a
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

/// How many ids a merge reads and inserts under one `vector_storage` read
/// guard ([`Collection::index_current`]).
pub(crate) const MERGE_CHUNK: usize = 256;

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

/// The unindexed ids — the async builder's and the deferred indexer's, each
/// once, sorted — read before an index search.
pub(crate) struct PendingSnapshot(Vec<u64>);

impl PendingSnapshot {
    /// A snapshot naming `ids` (sorted, each once), for a test.
    #[cfg(test)]
    pub(crate) fn for_test(mut ids: Vec<u64>) -> Self {
        ids.sort_unstable();
        ids.dedup();
        Self(ids)
    }

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
    /// them leave fewer than `k` others — `index_search` run again for more.
    /// The merge drops the index's result for an unindexed id, so without the
    /// refetch an id the index still holds, overwritten since, would take the
    /// place of the next point the index ranks rather than leave it to that
    /// point.
    ///
    /// Each refetch asks for at least twice as many as the last (and for `k`
    /// plus the unindexed ids just seen, if more), never more than `k` plus
    /// the snapshot's size, which no unindexed ids can crowd out. So it runs
    /// none unless an unindexed id is among the results, and at most
    /// `ceil(log2((k + |snapshot|) / k))` times: 4 for 100 crowding ids at
    /// `k = 10`.
    ///
    /// A refetch for `n` results needs all `n` from the index, not `k`: up to
    /// `n - k` of them may be unindexed ids the merge drops. So the index
    /// search a refetch runs is asked for `n` live results, and walks past
    /// nodes a delete or an upsert left behind until it has them.
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
        let cap = k.saturating_add(pending.0.len());
        let mut fetched = k;
        loop {
            let masked = results.iter().filter(|r| pending.holds(r.id)).count();
            // Fewer than asked for: the index has no more to give.
            if masked == 0 || results.len() < fetched || results.len() - masked >= k {
                return Ok(results);
            }
            let wanted = fetched
                .saturating_mul(2)
                .max(k.saturating_add(masked))
                .min(cap);
            if wanted <= fetched {
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

    /// Indexes what storage holds now for `ids` and returns the ids it
    /// settled: inserted, already held by the graph as storage holds them
    /// (`HnswIndex::insert_batch_unless_indexed`), no longer in storage, or
    /// holding a vector of the wrong dimension, which no merge could index
    /// (logged, and left out of the index rather than re-queued forever).
    /// An id it could not read or insert is not returned, so the merge that
    /// asked keeps it queued; one warning counts those, whatever their number.
    ///
    /// Each chunk of [`MERGE_CHUNK`] ids is read and inserted under one
    /// `vector_storage` (position 2) read guard. A delete or an upsert writes
    /// storage under the write guard before it touches the index (the order
    /// `delete_vector_core_stores` holds it in), so it lands wholly before
    /// the chunk is read, and the merge indexes what it left, or after the
    /// chunk is inserted, and its own index write comes last: a merge never
    /// puts a deleted point back nor overwrites a newer write with the
    /// vector it read. A writer waits at most one chunk's insert. A chunk the
    /// graph refuses is all-or-nothing, so its ids are retried one at a time,
    /// each read and inserted under a guard of its own: the retries never
    /// hold a writer for longer than one vector's insert.
    pub(crate) fn index_current(&self, ids: &[u64]) -> Vec<u64> {
        let dimension = self.storage.config.read().dimension;
        let mut settled = Vec::with_capacity(ids.len());
        let mut kept = KeptIds::default();
        for chunk in ids.chunks(MERGE_CHUNK) {
            let refused = self.index_chunk(chunk, dimension, &mut settled, &mut kept);
            if chunk.len() == 1 {
                kept.refused(&refused);
                continue;
            }
            for id in refused.ids {
                let refused = self.index_chunk(&[id], dimension, &mut settled, &mut kept);
                kept.refused(&refused);
            }
        }
        kept.report(ids.len());
        settled
    }

    /// Removes `ids` from the HNSW index only (`internal_bench::unindex`).
    #[cfg(any(test, feature = "internal-bench"))]
    pub(crate) fn remove_from_index_for_bench(&self, ids: &[u64]) {
        for &id in ids {
            self.storage.index.remove(id);
        }
    }

    /// Reads `chunk` from storage and inserts it, under one `vector_storage`
    /// read guard; settles what [`index_current`](Self::index_current)
    /// settles into `settled`, counts what it could not read into `kept`, and
    /// returns what the graph refused.
    fn index_chunk(
        &self,
        chunk: &[u64],
        dimension: usize,
        settled: &mut Vec<u64>,
        kept: &mut KeptIds,
    ) -> Refused {
        let storage = self.storage.vector_storage.read();
        #[cfg(feature = "internal-bench")]
        let held_since = std::time::Instant::now();
        let mut present = Vec::with_capacity(chunk.len());
        for &id in chunk {
            let read = storage.retrieve(id);
            #[cfg(test)]
            let read = merge_hooks::read(id, read);
            match read {
                Ok(Some(vector)) if vector.len() == dimension => present.push((id, vector)),
                Ok(Some(vector)) => {
                    tracing::error!(
                        "index_current: id {id} holds a {}-d vector in a {dimension}-d \
                         collection; left out of the index",
                        vector.len()
                    );
                    settled.push(id);
                }
                Ok(None) => settled.push(id),
                Err(e) => kept.unread(&e),
            }
        }
        #[cfg(test)]
        merge_hooks::after_read();
        #[cfg(test)]
        let outcome = merge_hooks::insert(&present)
            .unwrap_or_else(|| self.storage.index.insert_batch_unless_indexed(&present));
        #[cfg(not(test))]
        let outcome = self.storage.index.insert_batch_unless_indexed(&present);
        #[cfg(feature = "internal-bench")]
        crate::internal_bench::note_merge_chunk_hold(held_since.elapsed());
        drop(storage);
        settled.extend(outcome.settled);
        Refused {
            ids: outcome.refused,
            error: outcome.error,
        }
    }
}

/// The ids of a batch the graph refused, and why.
struct Refused {
    ids: Vec<u64>,
    error: Option<crate::error::Error>,
}

/// The ids one [`Collection::index_current`] could not settle, counted, with
/// the first reason of each kind, so a merge reports them in one warning
/// rather than one per id.
#[derive(Default)]
struct KeptIds {
    unread: usize,
    first_unread: Option<String>,
    refused: usize,
    first_refusal: Option<String>,
}

impl KeptIds {
    fn unread(&mut self, error: &std::io::Error) {
        self.unread += 1;
        self.first_unread.get_or_insert_with(|| error.to_string());
    }

    fn refused(&mut self, refused: &Refused) {
        self.refused += refused.ids.len();
        if let (None, Some(error)) = (&self.first_refusal, &refused.error) {
            self.first_refusal = Some(error.to_string());
        }
    }

    fn report(&self, asked: usize) {
        if self.unread == 0 && self.refused == 0 {
            return;
        }
        tracing::warn!(
            unread = self.unread,
            refused = self.refused,
            "index_current: {} of {asked} ids kept queued for the next merge: {} unread \
             (first: {}), {} refused by the graph (first: {})",
            self.unread + self.refused,
            self.unread,
            self.first_unread.as_deref().unwrap_or("-"),
            self.refused,
            self.first_refusal.as_deref().unwrap_or("-"),
        );
    }
}

/// A test's hook into a merge, run once a chunk's vectors are read and before
/// they are inserted: where a delete or an upsert racing the merge would
/// land if the merge let go of storage in between.
#[cfg(test)]
#[path = "pending_merge_hooks.rs"]
pub(crate) mod merge_hooks;
