//! Sparse inverted index with segment-level isolation.
//!
//! Implements a mutable + frozen segment architecture for concurrent
//! sparse vector insert, delete, and read operations.

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;
use rustc_hash::{FxHashMap, FxHashSet};

use super::types::{PostingEntry, SparseVector};

/// Number of documents in a mutable segment before it freezes.
pub const FREEZE_THRESHOLD: usize = 10_000;

/// The mutable (write-optimized) segment of the inverted index.
struct MutableSegment {
    postings: FxHashMap<u32, Vec<PostingEntry>>,
    max_weights: FxHashMap<u32, f32>,
    /// Set of all doc IDs currently held in this segment. Used to distinguish
    /// a true insert from an upsert (same `point_id`, updated weights) so that
    /// `doc_count` is only incremented for genuinely new documents.
    doc_set: FxHashSet<u64>,
    doc_count: usize,
}

impl MutableSegment {
    fn new() -> Self {
        Self {
            postings: FxHashMap::default(),
            max_weights: FxHashMap::default(),
            doc_set: FxHashSet::default(),
            doc_count: 0,
        }
    }

    /// Inserts or updates `vector` for `point_id`.
    ///
    /// Returns `true` if this is a new document (first time this `point_id`
    /// appears in this segment), `false` if it is an in-place update (upsert).
    /// Callers must only increment `doc_count` on the outer index when `true`
    /// is returned, to avoid double-counting on upserts.
    fn insert(&mut self, point_id: u64, vector: &SparseVector) -> bool {
        let is_new = self.doc_set.insert(point_id);

        for (&term_id, &weight) in vector.indices.iter().zip(vector.values.iter()) {
            let entries = self.postings.entry(term_id).or_default();

            let entry = PostingEntry {
                doc_id: point_id,
                weight,
            };

            // Maintain sorted order by doc_id
            match entries.binary_search_by_key(&point_id, |e| e.doc_id) {
                Ok(pos) => entries[pos] = entry,
                Err(pos) => entries.insert(pos, entry),
            }

            // Update max_weight for this term
            let abs_weight = weight.abs();
            let max_w = self.max_weights.entry(term_id).or_insert(0.0);
            if abs_weight > *max_w {
                *max_w = abs_weight;
            }
        }

        if is_new {
            self.doc_count += 1;
        }

        is_new
    }

    /// Removes all posting entries for `point_id`.
    ///
    /// Returns `true` if the point had at least one entry in this segment
    /// (i.e. was actually present and removed), `false` if it was not found.
    /// Also recalculates `max_weights` only for the terms that were modified.
    fn delete(&mut self, point_id: u64) -> bool {
        // Remove from doc_set so a subsequent re-insert is counted as a new doc.
        self.doc_set.remove(&point_id);

        let mut any_removed = false;
        // Terms that still have remaining entries after removal — need max_weight
        // recalculation.
        let mut recalc_terms: Vec<u32> = Vec::new();
        let mut empty_terms: Vec<u32> = Vec::new();

        for (&term_id, entries) in &mut self.postings {
            let before = entries.len();
            entries.retain(|e| e.doc_id != point_id);
            if entries.len() < before {
                any_removed = true;
                if entries.is_empty() {
                    empty_terms.push(term_id);
                } else {
                    recalc_terms.push(term_id);
                }
            }
        }

        // Remove posting lists that became empty.
        for term_id in &empty_terms {
            self.postings.remove(term_id);
            self.max_weights.remove(term_id);
        }

        // Recalculate max_weights ONLY for terms that lost an entry but still
        // have remaining postings. This is O(postings for modified terms only),
        // not O(total_postings).
        for term_id in recalc_terms {
            if let Some(entries) = self.postings.get(&term_id) {
                let max_w = entries
                    .iter()
                    .map(|e| e.weight.abs())
                    .fold(0.0_f32, f32::max);
                self.max_weights.insert(term_id, max_w);
            }
        }

        any_removed
    }
}

/// A frozen (read-optimized) segment. The `f32` in the tuple is `max_weight`.
///
/// `pub(crate)` fields are consumed exclusively by `index::sparse::persistence`,
/// which is gated behind `feature = "persistence"`. Without that feature the
/// compiler cannot see those usages, so the lint is suppressed here rather than
/// at the module level to keep the scope as narrow as possible.
#[allow(dead_code)]
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
    pub(crate) fn new(
        postings: FxHashMap<u32, (Vec<PostingEntry>, f32)>,
        doc_count: usize,
    ) -> Self {
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
    #[must_use]
    pub fn get_all_postings(&self, term_id: u32) -> Vec<PostingEntry> {
        let mut result = Vec::new();

        // Read from frozen segments first
        {
            let frozen_vec = self.frozen.read();
            for frozen_seg in frozen_vec.iter() {
                if let Some((entries, _)) = frozen_seg.postings.get(&term_id) {
                    for entry in entries {
                        if !frozen_seg.tombstones.contains(&entry.doc_id) {
                            result.push(*entry);
                        }
                    }
                }
            }
        }

        // Read from mutable segment
        {
            let seg = self.mutable.read();
            if let Some(entries) = seg.postings.get(&term_id) {
                result.extend_from_slice(entries);
            }
        }

        // Sort merged results by doc_id
        result.sort_by_key(|e| e.doc_id);
        result
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

    /// Returns the number of unique terms across all segments.
    #[must_use]
    pub fn term_count(&self) -> usize {
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

        terms.len()
    }

    /// Constructs an index from a single frozen segment (used by persistence layer).
    ///
    /// Only called from `index::sparse::persistence` (feature = "persistence").
    #[must_use]
    #[allow(dead_code)]
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

        let mut ids: Vec<u32> = terms.into_iter().collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vector(pairs: Vec<(u32, f32)>) -> SparseVector {
        SparseVector::new(pairs)
    }

    #[test]
    fn test_insert_creates_posting_lists() {
        let index = SparseInvertedIndex::new();
        let v = make_vector(vec![(1, 0.5), (3, 1.0), (7, 0.3)]);
        index.insert(100, &v);

        assert_eq!(index.doc_count(), 1);

        let postings_1 = index.get_all_postings(1);
        assert_eq!(postings_1.len(), 1);
        assert_eq!(postings_1[0].doc_id, 100);
        assert!((postings_1[0].weight - 0.5).abs() < f32::EPSILON);

        let postings_3 = index.get_all_postings(3);
        assert_eq!(postings_3.len(), 1);
        assert_eq!(postings_3[0].doc_id, 100);

        let postings_7 = index.get_all_postings(7);
        assert_eq!(postings_7.len(), 1);
    }

    #[test]
    fn test_insert_updates_max_weight() {
        let index = SparseInvertedIndex::new();
        let v1 = make_vector(vec![(1, 0.5)]);
        let v2 = make_vector(vec![(1, 2.0)]);
        index.insert(1, &v1);
        index.insert(2, &v2);

        assert!((index.get_global_max_weight(1) - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_freeze_at_threshold() {
        let index = SparseInvertedIndex::new();
        for i in 0..=FREEZE_THRESHOLD {
            let v = make_vector(vec![(1, 1.0)]);
            index.insert(i as u64, &v);
        }

        assert_eq!(index.frozen_count(), 1);
        assert_eq!(index.mutable_doc_count(), 1);
        assert_eq!(index.doc_count(), (FREEZE_THRESHOLD + 1) as u64);
    }

    #[test]
    fn test_read_across_segments() {
        let index = SparseInvertedIndex::new();

        // Fill up to freeze
        for i in 0..FREEZE_THRESHOLD {
            let v = make_vector(vec![(1, 1.0)]);
            index.insert(i as u64, &v);
        }
        assert_eq!(index.frozen_count(), 1);

        // Insert into new mutable segment
        let v = make_vector(vec![(1, 2.0)]);
        index.insert(99_999, &v);

        let postings = index.get_all_postings(1);
        // FREEZE_THRESHOLD from frozen + 1 from mutable
        assert_eq!(postings.len(), FREEZE_THRESHOLD + 1);
    }

    #[test]
    fn test_delete_from_mutable() {
        let index = SparseInvertedIndex::new();
        let v = make_vector(vec![(1, 1.0), (2, 2.0)]);
        index.insert(42, &v);

        let postings = index.get_all_postings(1);
        assert_eq!(postings.len(), 1);

        index.delete(42);

        let postings = index.get_all_postings(1);
        assert!(postings.is_empty());
        let postings = index.get_all_postings(2);
        assert!(postings.is_empty());
    }

    #[test]
    fn test_delete_from_frozen_uses_tombstone() {
        let index = SparseInvertedIndex::new();

        // Fill to freeze
        for i in 0..FREEZE_THRESHOLD {
            let v = make_vector(vec![(1, 1.0)]);
            index.insert(i as u64, &v);
        }
        assert_eq!(index.frozen_count(), 1);

        // Delete doc 0 from frozen segment
        index.delete(0);

        let postings = index.get_all_postings(1);
        assert_eq!(postings.len(), FREEZE_THRESHOLD - 1);
        assert!(!postings.iter().any(|e| e.doc_id == 0));
    }

    #[test]
    fn test_get_max_weight_across_segments() {
        let index = SparseInvertedIndex::new();

        // Insert a vector with weight 5.0 for term 1, fill to freeze
        let v = make_vector(vec![(1, 5.0)]);
        index.insert(0, &v);
        for i in 1..FREEZE_THRESHOLD {
            let v = make_vector(vec![(1, 1.0)]);
            index.insert(i as u64, &v);
        }
        assert_eq!(index.frozen_count(), 1);

        // Insert into mutable with weight 3.0
        let v = make_vector(vec![(1, 3.0)]);
        index.insert(99_999, &v);

        // Max should be 5.0 from frozen segment
        assert!((index.get_global_max_weight(1) - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_term_count() {
        let index = SparseInvertedIndex::new();
        let v1 = make_vector(vec![(1, 1.0), (2, 2.0)]);
        let v2 = make_vector(vec![(2, 1.0), (3, 3.0)]);
        index.insert(1, &v1);
        index.insert(2, &v2);

        assert_eq!(index.term_count(), 3); // terms 1, 2, 3
    }

    #[test]
    fn test_concurrent_insert() {
        use std::sync::Arc;

        let index = Arc::new(SparseInvertedIndex::new());
        let mut handles = Vec::new();

        for thread_id in 0..4u64 {
            let idx = Arc::clone(&index);
            handles.push(std::thread::spawn(move || {
                for i in 0..100u64 {
                    let point_id = thread_id * 1000 + i;
                    let v = SparseVector::new(vec![(1, 1.0), (2, 0.5)]);
                    idx.insert(point_id, &v);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(index.doc_count(), 400);
    }

    #[test]
    fn test_empty_index() {
        let index = SparseInvertedIndex::new();
        assert_eq!(index.doc_count(), 0);
        assert_eq!(index.term_count(), 0);
        assert!(index.get_all_postings(1).is_empty());
        assert!((index.get_global_max_weight(1)).abs() < f32::EPSILON);
    }

    // --- Bug-fix regression tests ---

    #[test]
    fn test_double_delete_no_underflow() {
        let index = SparseInvertedIndex::new();
        index.insert(42, &make_vector(vec![(1, 1.0)]));
        assert_eq!(index.doc_count(), 1);

        index.delete(42);
        assert_eq!(index.doc_count(), 0);

        // Second delete of the same point must not wrap to u64::MAX.
        index.delete(42);
        assert_eq!(
            index.doc_count(),
            0,
            "doc_count must not underflow on double-delete"
        );
    }

    #[test]
    fn test_delete_nonexistent_no_underflow() {
        let index = SparseInvertedIndex::new();
        assert_eq!(index.doc_count(), 0);

        // Deleting a point that was never inserted must leave count at 0.
        index.delete(999);
        assert_eq!(
            index.doc_count(),
            0,
            "doc_count must not underflow on delete of non-existent id"
        );
    }

    #[test]
    fn test_upsert_same_id_does_not_increment_doc_count() {
        // H-3 regression: inserting the same point_id twice must not double-count.
        let index = SparseInvertedIndex::new();
        let v1 = make_vector(vec![(1, 1.0)]);
        let v2 = make_vector(vec![(1, 2.0)]);

        index.insert(42, &v1);
        assert_eq!(index.doc_count(), 1, "first insert must set doc_count to 1");

        // Upsert same ID with updated weight — doc_count must stay at 1.
        index.insert(42, &v2);
        assert_eq!(
            index.doc_count(),
            1,
            "upsert of existing ID must not increment doc_count"
        );

        // Weight must reflect the latest insert (upsert semantics).
        let postings = index.get_all_postings(1);
        assert_eq!(postings.len(), 1);
        assert!(
            (postings[0].weight - 2.0).abs() < f32::EPSILON,
            "upsert must update the stored weight"
        );
    }

    #[test]
    fn test_upsert_different_terms_does_not_increment_doc_count() {
        // Upsert where the new vector uses different terms than the first insert.
        let index = SparseInvertedIndex::new();

        index.insert(99, &make_vector(vec![(10, 1.0)]));
        assert_eq!(index.doc_count(), 1);

        // Same point_id, completely different term set.
        index.insert(99, &make_vector(vec![(20, 0.5)]));
        assert_eq!(
            index.doc_count(),
            1,
            "upsert with different terms must not increment doc_count"
        );
    }

    #[test]
    fn test_dedup_last_write_wins_within_mutable() {
        // Insert point 1 twice; second insert (upsert) updates in-place via
        // binary_search. Compaction must see only the newer weight.
        let index = SparseInvertedIndex::new();
        index.insert(1, &make_vector(vec![(5, 0.1)]));
        index.insert(1, &make_vector(vec![(5, 9.9)]));

        let compacted = index.get_merged_postings_for_compaction();
        let (entries, _) = compacted.get(&5).expect("term 5 must be present");
        let entry = entries
            .iter()
            .find(|e| e.doc_id == 1)
            .expect("doc 1 must be present");
        assert!(
            (entry.weight - 9.9).abs() < 1e-5,
            "compaction must keep newest weight; got {}",
            entry.weight
        );
    }

    #[test]
    fn test_dedup_last_write_wins_across_segments() {
        // Force doc 0 into a frozen segment, then re-insert it with a different
        // weight in the mutable segment. Compaction must pick the mutable weight.
        let index = SparseInvertedIndex::new();

        for i in 0..FREEZE_THRESHOLD {
            index.insert(i as u64, &make_vector(vec![(7, 1.0)]));
        }
        assert_eq!(index.frozen_count(), 1, "segment must have frozen");

        // Re-insert doc 0 into the mutable segment with an updated weight.
        index.insert(0, &make_vector(vec![(7, 5.5)]));

        let compacted = index.get_merged_postings_for_compaction();
        let (entries, _) = compacted.get(&7).expect("term 7 must be present");
        let entry = entries
            .iter()
            .find(|e| e.doc_id == 0)
            .expect("doc 0 must be present");
        assert!(
            (entry.weight - 5.5).abs() < 1e-5,
            "mutable (newer) weight 5.5 must win over frozen weight 1.0; got {}",
            entry.weight
        );
    }
}
