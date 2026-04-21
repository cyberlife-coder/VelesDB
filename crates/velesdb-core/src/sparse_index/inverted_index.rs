//! Sparse inverted index with segment-level isolation.
//!
//! Implements a mutable + frozen segment architecture for concurrent
//! sparse vector insert, delete, and read operations.

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;
use rustc_hash::{FxHashMap, FxHashSet};

use super::mutable_segment::MutableSegment;
use super::types::{PostingEntry, SparseVector};

/// Number of documents in a mutable segment before it freezes.
pub const FREEZE_THRESHOLD: usize = 10_000;

// MutableSegment is in mutable_segment.rs

/// A frozen (read-optimized) segment. The `f32` in the tuple is `max_weight`.
///
/// # Invariants
///
/// Every posting list in `postings` is sorted ascending by `doc_id`. This
/// is required by [`SparseInvertedIndex::get_all_postings`] (which feeds
/// a k-way merge over the per-segment runs) and by
/// [`super::search::strategy::linear_scan_search`] and
/// `maxscore_search` (which assume sorted cursors). [`MutableSegment`]
/// enforces the invariant via binary-search insert in `insert` and the
/// merge-join in `merge_batch_postings`; `freeze_inner` moves the
/// already-sorted posting lists into the frozen segment as-is. The
/// persistence layer must preserve it when loading from disk — see the
/// debug assertion in [`Self::new`].
///
/// `pub(crate)` fields are consumed exclusively by `index::sparse::persistence`,
/// which is gated behind `feature = "persistence"`. Without that feature the
/// compiler cannot see those usages, so the lint is suppressed here rather than
/// at the module level to keep the scope as narrow as possible.
#[allow(dead_code)] // Reason: Used by sparse::persistence behind `feature = "persistence"`
pub(crate) struct FrozenSegment {
    /// Posting lists per term. The `f32` is the max absolute weight for that term.
    pub(crate) postings: FxHashMap<u32, (Vec<PostingEntry>, f32)>,
    /// Tombstone set: doc IDs that have been logically deleted.
    tombstones: FxHashSet<u64>,
    /// Number of documents originally in this segment (used by persistence layer).
    pub(crate) doc_count: usize,
}

impl FrozenSegment {
    /// Creates a new frozen segment from postings and a document count.
    ///
    /// In debug builds, asserts the "sorted ascending by `doc_id`" invariant
    /// documented on the struct. The check runs only under
    /// `debug_assertions`, so release builds pay no cost. Persistence
    /// layers that synthesise `FrozenSegment` values therefore exercise
    /// the assertion during local / CI test runs before shipping a release
    /// binary that relies on the invariant.
    #[allow(dead_code)] // Reason: Used by sparse::persistence behind `feature = "persistence"`
    pub(crate) fn new(
        postings: FxHashMap<u32, (Vec<PostingEntry>, f32)>,
        doc_count: usize,
    ) -> Self {
        debug_assert!(
            postings
                .iter()
                .all(|(_, (entries, _))| entries.windows(2).all(|w| w[0].doc_id < w[1].doc_id)),
            "FrozenSegment posting lists must be sorted ascending and deduplicated by doc_id"
        );
        Self {
            postings,
            tombstones: FxHashSet::default(),
            doc_count,
        }
    }

    /// Returns `true` if `point_id` has a live (non-tombstoned) entry in this segment.
    fn contains_live(&self, point_id: u64) -> bool {
        if self.tombstones.contains(&point_id) {
            return false;
        }
        self.postings.values().any(|(entries, _)| {
            entries
                .binary_search_by_key(&point_id, |e| e.doc_id)
                .is_ok()
        })
    }
}

/// Sparse inverted index with segment-level isolation.
///
/// Uses a single `RwLock<MutableSegment>` for writes and a
/// `RwLock<Vec<FrozenSegment>>` for frozen segments. When the mutable
/// segment reaches [`FREEZE_THRESHOLD`] documents it is frozen and a new
/// empty mutable segment is created.
///
/// # Lock ordering
///
/// When both locks are needed, always acquire `mutable` before `frozen`
/// (lock order position 9 in the canonical ordering).
pub struct SparseInvertedIndex {
    mutable: RwLock<MutableSegment>,
    frozen: RwLock<Vec<FrozenSegment>>,
    doc_count: AtomicU64,
}

impl Default for SparseInvertedIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseInvertedIndex {
    /// Creates a new, empty sparse inverted index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mutable: RwLock::new(MutableSegment::new()),
            frozen: RwLock::new(Vec::new()),
            doc_count: AtomicU64::new(0),
        }
    }

    /// Inserts or updates a sparse vector for the given point ID.
    ///
    /// `doc_count` is incremented only when this is a genuinely new document.
    /// Re-inserting the same `point_id` (upsert) updates the posting weights
    /// in-place without incrementing the count.
    ///
    /// If the mutable segment reaches [`FREEZE_THRESHOLD`] documents,
    /// it is automatically frozen into an immutable segment.
    pub fn insert(&self, point_id: u64, vector: &SparseVector) {
        let mut seg = self.mutable.write();
        let is_new = seg.insert(point_id, vector);
        if is_new {
            self.doc_count.fetch_add(1, Ordering::Relaxed);
        }

        if seg.doc_count >= FREEZE_THRESHOLD {
            self.freeze_inner(&mut seg);
        }
    }

    /// Inserts a batch of sparse vectors while acquiring the mutable lock once.
    ///
    /// Preserves the current per-term upsert semantics of repeated `insert()`:
    /// later entries in the batch overwrite earlier entries for the same
    /// `(term_id, doc_id)` pair, while untouched terms from prior inserts remain.
    // Reason: called from `collection::core::crud::upsert_bulk` and `internal_bench::sparse_insert_batch`.
    // The dead_code lint has a false positive because the call site reaches this method through
    // a `RwLockWriteGuard<BTreeMap<_,SparseInvertedIndex>>` deref chain which the lint does not trace.
    #[allow(dead_code)] // Reason: False positive — called via RwLockWriteGuard deref chain
    pub(crate) fn insert_batch_chunk(&self, docs: &[(u64, SparseVector)]) {
        if docs.is_empty() {
            return;
        }

        let (batch_postings, batch_max_weights, batch_doc_ids) = Self::build_batch_buffers(docs);

        let mut seg = self.mutable.write();
        let new_docs = Self::merge_doc_ids(&mut seg, batch_doc_ids);
        if new_docs > 0 {
            self.doc_count.fetch_add(new_docs, Ordering::Relaxed);
        }

        Self::merge_batch_into_segment(&mut seg, batch_postings, &batch_max_weights);

        if seg.doc_count >= FREEZE_THRESHOLD {
            self.freeze_inner(&mut seg);
        }
    }

    /// Pre-computes posting entries and max weights from a batch of documents.
    #[allow(clippy::type_complexity)]
    fn build_batch_buffers(
        docs: &[(u64, SparseVector)],
    ) -> (
        FxHashMap<u32, Vec<PostingEntry>>,
        FxHashMap<u32, f32>,
        FxHashSet<u64>,
    ) {
        let mut batch_postings: FxHashMap<u32, Vec<PostingEntry>> = FxHashMap::default();
        let mut batch_max_weights: FxHashMap<u32, f32> = FxHashMap::default();
        let mut batch_doc_ids: FxHashSet<u64> = FxHashSet::default();

        for (point_id, vector) in docs {
            batch_doc_ids.insert(*point_id);
            for (&term_id, &weight) in vector.indices.iter().zip(vector.values.iter()) {
                batch_postings
                    .entry(term_id)
                    .or_default()
                    .push(PostingEntry {
                        doc_id: *point_id,
                        weight,
                    });
                let abs_weight = weight.abs();
                let max_weight = batch_max_weights.entry(term_id).or_insert(0.0);
                if abs_weight > *max_weight {
                    *max_weight = abs_weight;
                }
            }
        }

        (batch_postings, batch_max_weights, batch_doc_ids)
    }

    /// Merges batch doc IDs into the mutable segment, returning the count of new docs.
    fn merge_doc_ids(seg: &mut MutableSegment, batch_doc_ids: FxHashSet<u64>) -> u64 {
        let mut new_docs = 0_u64;
        for point_id in batch_doc_ids {
            if seg.doc_set.insert(point_id) {
                seg.doc_count += 1;
                new_docs += 1;
            }
        }
        new_docs
    }

    /// Merges batch postings and max weights into the mutable segment.
    fn merge_batch_into_segment(
        seg: &mut MutableSegment,
        batch_postings: FxHashMap<u32, Vec<PostingEntry>>,
        batch_max_weights: &FxHashMap<u32, f32>,
    ) {
        for (term_id, updates) in batch_postings {
            let entries = seg.postings.entry(term_id).or_default();
            MutableSegment::merge_batch_postings(entries, updates);

            if let Some(&abs_weight) = batch_max_weights.get(&term_id) {
                let max_weight = seg.max_weights.entry(term_id).or_insert(0.0);
                if abs_weight > *max_weight {
                    *max_weight = abs_weight;
                }
            }
        }
    }

    /// Freezes the current mutable segment (must be called with write lock held).
    fn freeze_inner(&self, seg: &mut MutableSegment) {
        let old = std::mem::replace(seg, MutableSegment::new());

        // Convert MutableSegment -> FrozenSegment
        let mut frozen_postings = FxHashMap::default();
        for (term_id, entries) in old.postings {
            let max_w = old.max_weights.get(&term_id).copied().unwrap_or(0.0);
            frozen_postings.insert(term_id, (entries, max_w));
        }

        let frozen_seg = FrozenSegment {
            postings: frozen_postings,
            tombstones: FxHashSet::default(),
            doc_count: old.doc_count,
        };

        let mut frozen_vec = self.frozen.write();
        frozen_vec.push(frozen_seg);
    }

    /// Deletes a point from the index.
    ///
    /// Removes entries from the mutable segment and adds a tombstone to frozen
    /// segments that actually contain the point as a live entry.
    /// `doc_count` is decremented **only** if the point was actually present,
    /// preventing underflow on double-delete or delete of a non-existent ID.
    pub fn delete(&self, point_id: u64) {
        // Lock ordering: mutable before frozen (position 9 in canonical order).
        let mut seg = self.mutable.write();
        let was_in_mutable = seg.delete(point_id);

        let mut frozen_vec = self.frozen.write();
        let mut was_in_frozen = false;
        for frozen_seg in frozen_vec.iter_mut() {
            // Only insert a tombstone — and only count as "found" — if the
            // point was live in this segment (not already tombstoned).
            if frozen_seg.contains_live(point_id) {
                frozen_seg.tombstones.insert(point_id);
                was_in_frozen = true;
            }
        }

        if was_in_mutable || was_in_frozen {
            self.doc_count.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Returns the approximate document count in this index.
    ///
    /// Uses `Relaxed` ordering intentionally: this count is only used for the
    /// heuristic branch selection between `MaxScore` and linear scan in search.
    /// Stale counts cause at most a suboptimal algorithm choice, not incorrect
    /// results. Actual mutations are protected by write locks on the segments.
    #[must_use]
    pub fn doc_count(&self) -> u64 {
        self.doc_count.load(Ordering::Relaxed)
    }

    /// Returns the number of live posting entries for a term without materialising
    /// the full `Vec`. Use this in coverage heuristics to avoid a throwaway
    /// allocation; call `get_all_postings` only when the entries themselves are needed.
    ///
    /// # Lock ordering note
    ///
    /// Write paths (`insert`, `delete`, `freeze_inner`) acquire `mutable.write()`
    /// before `frozen.write()` — the canonical ordering. Read paths (`posting_count`,
    /// `get_all_postings`) acquire `frozen.read()` then `mutable.read()` in reverse
    /// order. This is safe because `parking_lot` read locks are non-exclusive and
    /// cannot create a deadlock cycle with the write path. If this code is ever
    /// restructured to hold a write lock on one while acquiring the other in the read
    /// path, it MUST be updated to match the canonical mutable-before-frozen ordering.
    #[must_use]
    pub fn posting_count(&self, term_id: u32) -> usize {
        let mut count: usize = 0;

        {
            let frozen_vec = self.frozen.read();
            for frozen_seg in frozen_vec.iter() {
                if let Some((entries, _)) = frozen_seg.postings.get(&term_id) {
                    count += entries
                        .iter()
                        .filter(|e| !frozen_seg.tombstones.contains(&e.doc_id))
                        .count();
                }
            }
        }

        {
            let seg = self.mutable.read();
            if let Some(entries) = seg.postings.get(&term_id) {
                count += entries.len();
            }
        }

        count
    }

    /// Returns all posting entries for a term across all segments,
    /// filtering tombstoned entries. Result is sorted by `doc_id`.
    ///
    /// Exploits the per-segment invariant (`postings[term_id]` is already
    /// sorted by `doc_id` in both mutable and frozen segments) to perform a
    /// k-way merge in `O(N · M)` instead of the general `O(N log N)` sort
    /// previously used — a ~5x reduction in the hot-path work when
    /// `M ∈ {1, 2}` (the common case for a single-freeze corpus).
    #[must_use]
    pub fn get_all_postings(&self, term_id: u32) -> Vec<PostingEntry> {
        let frozen_runs = self.collect_frozen_runs(term_id);
        let mutable_run = self.collect_mutable_run(term_id);
        merge_sorted_runs(frozen_runs, mutable_run)
    }

    /// Snapshots each frozen segment's tombstone-filtered posting list for
    /// `term_id`, cloning owned data so the `frozen` read lock is released
    /// before this function returns.
    ///
    /// Entries are cloned out of the lock scope so the subsequent merge
    /// runs without any lock held — this prevents the ABBA deadlock that
    /// would otherwise occur if the reader held both `frozen.read()` and
    /// `mutable.read()` simultaneously while a concurrent writer held
    /// `mutable.write()` and blocked on `frozen.write()`.
    #[inline]
    fn collect_frozen_runs(&self, term_id: u32) -> Vec<Vec<PostingEntry>> {
        let frozen_vec = self.frozen.read();
        frozen_vec
            .iter()
            .filter_map(|seg| {
                let (entries, _) = seg.postings.get(&term_id)?;
                if entries.is_empty() {
                    return None;
                }
                let owned: Vec<PostingEntry> = if seg.tombstones.is_empty() {
                    entries.clone()
                } else {
                    entries
                        .iter()
                        .copied()
                        .filter(|e| !seg.tombstones.contains(&e.doc_id))
                        .collect()
                };
                (!owned.is_empty()).then_some(owned)
            })
            .collect()
    }

    /// Snapshots the mutable segment's posting list for `term_id`, cloning
    /// owned data so the `mutable` read lock is released before this
    /// function returns. See [`Self::collect_frozen_runs`] for the
    /// lock-ordering rationale.
    #[inline]
    fn collect_mutable_run(&self, term_id: u32) -> Vec<PostingEntry> {
        let seg = self.mutable.read();
        seg.postings.get(&term_id).cloned().unwrap_or_default()
    }

    /// Returns the maximum absolute weight for a term across all segments.
    #[must_use]
    pub fn get_global_max_weight(&self, term_id: u32) -> f32 {
        let mut max_w = 0.0_f32;

        {
            let frozen_vec = self.frozen.read();
            for frozen_seg in frozen_vec.iter() {
                if let Some(&(_, w)) = frozen_seg.postings.get(&term_id) {
                    max_w = max_w.max(w);
                }
            }
        }

        {
            let seg = self.mutable.read();
            if let Some(&w) = seg.max_weights.get(&term_id) {
                max_w = max_w.max(w);
            }
        }

        max_w
    }

    /// Collects all unique term IDs from frozen and mutable segments into a set.
    fn collect_term_ids(&self) -> FxHashSet<u32> {
        let mut terms: FxHashSet<u32> = FxHashSet::default();

        {
            let frozen_vec = self.frozen.read();
            for frozen_seg in frozen_vec.iter() {
                terms.extend(frozen_seg.postings.keys());
            }
        }

        {
            let seg = self.mutable.read();
            terms.extend(seg.postings.keys());
        }

        terms
    }

    /// Returns the number of unique terms across all segments.
    #[must_use]
    pub fn term_count(&self) -> usize {
        self.collect_term_ids().len()
    }

    /// Constructs an index from a single frozen segment (used by persistence layer).
    ///
    /// Only called from `index::sparse::persistence` (feature = "persistence").
    #[must_use]
    #[allow(dead_code)] // Reason: Called from sparse::persistence behind `feature = "persistence"`
    pub(crate) fn from_frozen_segment(segment: FrozenSegment) -> Self {
        let doc_count = segment.doc_count as u64;
        Self {
            mutable: RwLock::new(MutableSegment::new()),
            frozen: RwLock::new(vec![segment]),
            doc_count: AtomicU64::new(doc_count),
        }
    }

    /// Returns all unique term IDs across all segments.
    #[must_use]
    pub fn all_term_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.collect_term_ids().into_iter().collect();
        ids.sort_unstable();
        ids
    }

    /// Merges all segments into a single map for disk compaction.
    ///
    /// Filters tombstoned entries and recalculates max weights.
    /// Returns `(term_id -> (postings, max_weight))`.
    ///
    /// ## Last-write-wins semantics
    ///
    /// The mutable segment holds the newest writes; frozen segments are older.
    /// To preserve newest-wins when the same `doc_id` appears in both, mutable
    /// entries are inserted **first** into the per-term buffer. After a stable
    /// sort by `doc_id`, each mutable entry precedes any same-id frozen entry,
    /// so `dedup_by_key` (which retains the first occurrence) keeps the mutable
    /// (newer) weight.
    #[must_use]
    pub fn get_merged_postings_for_compaction(&self) -> FxHashMap<u32, (Vec<PostingEntry>, f32)> {
        let mut merged: FxHashMap<u32, Vec<PostingEntry>> = FxHashMap::default();

        // Insert mutable entries FIRST — they are the newest writes.
        {
            let seg = self.mutable.read();
            for (&term_id, entries) in &seg.postings {
                let dest = merged.entry(term_id).or_default();
                dest.extend_from_slice(entries);
            }
        }

        // Append frozen entries (older), filtering tombstones.
        {
            let frozen_vec = self.frozen.read();
            for frozen_seg in frozen_vec.iter() {
                for (&term_id, (entries, _)) in &frozen_seg.postings {
                    let dest = merged.entry(term_id).or_default();
                    for entry in entries {
                        if !frozen_seg.tombstones.contains(&entry.doc_id) {
                            dest.push(*entry);
                        }
                    }
                }
            }
        }

        // Sort by doc_id then dedup, keeping the first occurrence per doc_id.
        // Because mutable entries were inserted before frozen entries, the first
        // occurrence of any doc_id that appears in both segments is the mutable
        // (newer) one — last-write-wins is correctly enforced.
        let mut result: FxHashMap<u32, (Vec<PostingEntry>, f32)> = FxHashMap::default();
        for (term_id, mut entries) in merged {
            entries.sort_by_key(|e| e.doc_id);
            entries.dedup_by_key(|e| e.doc_id);

            if entries.is_empty() {
                continue;
            }

            let max_w = entries
                .iter()
                .map(|e| e.weight.abs())
                .fold(0.0_f32, f32::max);
            result.insert(term_id, (entries, max_w));
        }

        result
    }

    /// Returns the number of frozen segments (for testing).
    #[cfg(test)]
    fn frozen_count(&self) -> usize {
        self.frozen.read().len()
    }

    /// Returns the mutable segment doc count (for testing).
    #[cfg(test)]
    fn mutable_doc_count(&self) -> usize {
        self.mutable.read().doc_count
    }
}

/// Merges several `doc_id`-sorted posting runs into a single sorted vector.
///
/// Consumes its inputs and runs without holding any index lock — callers
/// are expected to snapshot the per-segment data via
/// [`SparseInvertedIndex::collect_frozen_runs`] and
/// [`SparseInvertedIndex::collect_mutable_run`] first.
///
/// Applies **last-write-wins** semantics when a `doc_id` appears in more
/// than one run: callers place mutable data after frozen data, so an
/// upsert that crossed a segment freeze contributes its mutable weight
/// exactly once instead of double-counting the stale frozen entry when
/// downstream consumers (e.g. `linear_scan_search`, `brute_force_search`)
/// sum posting weights into an accumulator map.
#[inline]
fn merge_sorted_runs(
    frozen_runs: Vec<Vec<PostingEntry>>,
    mutable_run: Vec<PostingEntry>,
) -> Vec<PostingEntry> {
    let mut runs: Vec<Vec<PostingEntry>> = frozen_runs;
    if !mutable_run.is_empty() {
        runs.push(mutable_run);
    }
    match runs.len() {
        0 => Vec::new(),
        1 => runs.into_iter().next().unwrap_or_default(),
        _ => k_way_merge(&runs),
    }
}

/// k-way merge of sorted runs with last-write-wins dedup.
///
/// On each iteration picks the smallest `doc_id` across all run heads,
/// then advances **every** cursor whose head matches that `doc_id`,
/// keeping the entry from the last such run. This coalesces
/// duplicate-`doc_id` entries produced by an upsert crossing a segment
/// freeze into a single output entry with the newest weight.
fn k_way_merge(runs: &[Vec<PostingEntry>]) -> Vec<PostingEntry> {
    let total_len: usize = runs.iter().map(Vec::len).sum();
    let mut result: Vec<PostingEntry> = Vec::with_capacity(total_len);
    let mut cursors: Vec<usize> = vec![0; runs.len()];
    while let Some(min_doc_id) = find_min_head_doc_id(runs, &cursors) {
        if let Some(entry) = advance_matching_runs(runs, &mut cursors, min_doc_id) {
            result.push(entry);
        }
    }
    result
}

/// Returns the smallest `doc_id` among the current heads of `runs`,
/// or `None` when every cursor has exhausted its run.
#[inline]
fn find_min_head_doc_id(runs: &[Vec<PostingEntry>], cursors: &[usize]) -> Option<u64> {
    runs.iter()
        .enumerate()
        .filter_map(|(i, run)| run.get(cursors[i]).map(|entry| entry.doc_id))
        .min()
}

/// Advances every cursor whose run-head matches `target_doc_id`, returning
/// the entry from the **last** such run for the last-write-wins dedup
/// documented on [`merge_sorted_runs`].
#[inline]
fn advance_matching_runs(
    runs: &[Vec<PostingEntry>],
    cursors: &mut [usize],
    target_doc_id: u64,
) -> Option<PostingEntry> {
    let mut picked: Option<PostingEntry> = None;
    for (i, run) in runs.iter().enumerate() {
        if run
            .get(cursors[i])
            .is_some_and(|entry| entry.doc_id == target_doc_id)
        {
            picked = Some(run[cursors[i]]);
            cursors[i] += 1;
        }
    }
    picked
}

#[cfg(test)]
#[path = "inverted_index_tests.rs"]
mod tests;
