//! Mutable (write-optimized) segment of the sparse inverted index.
//!
//! Extracted from `inverted_index.rs` to reduce NLOC below the 500 threshold.

use rustc_hash::{FxHashMap, FxHashSet};

use super::types::{PostingEntry, SparseVector};

/// The mutable (write-optimized) segment of the inverted index.
pub(super) struct MutableSegment {
    pub(super) postings: FxHashMap<u32, Vec<PostingEntry>>,
    pub(super) max_weights: FxHashMap<u32, f32>,
    /// Set of all doc IDs currently held in this segment.
    pub(super) doc_set: FxHashSet<u64>,
    pub(super) doc_count: usize,
}

impl MutableSegment {
    pub(super) fn new() -> Self {
        Self {
            postings: FxHashMap::default(),
            max_weights: FxHashMap::default(),
            doc_set: FxHashSet::default(),
            doc_count: 0,
        }
    }

    /// Inserts or updates `vector` for `point_id`.
    ///
    /// Returns `true` if this is a new document.
    pub(super) fn insert(&mut self, point_id: u64, vector: &SparseVector) -> bool {
        let is_new = self.doc_set.insert(point_id);

        for (&term_id, &weight) in vector.indices.iter().zip(vector.values.iter()) {
            let entries = self.postings.entry(term_id).or_default();

            let entry = PostingEntry {
                doc_id: point_id,
                weight,
            };

            match entries.binary_search_by_key(&point_id, |e| e.doc_id) {
                Ok(pos) => entries[pos] = entry,
                Err(pos) => entries.insert(pos, entry),
            }

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

    #[allow(dead_code)] // Reason: Called from inverted_index::insert_batch_chunk via deref chain
    pub(super) fn merge_batch_postings(
        entries: &mut Vec<PostingEntry>,
        mut updates: Vec<PostingEntry>,
    ) {
        if updates.is_empty() {
            return;
        }

        updates.sort_by_key(|entry| entry.doc_id);

        let mut deduped_rev = Vec::with_capacity(updates.len());
        for entry in updates.into_iter().rev() {
            if deduped_rev
                .last()
                .is_none_or(|last: &PostingEntry| last.doc_id != entry.doc_id)
            {
                deduped_rev.push(entry);
            }
        }
        deduped_rev.reverse();

        let existing = std::mem::take(entries);
        let mut merged = Vec::with_capacity(existing.len() + deduped_rev.len());
        let mut existing_iter = existing.into_iter().peekable();
        let mut updates_iter = deduped_rev.into_iter().peekable();

        while let (Some(existing_entry), Some(update_entry)) =
            (existing_iter.peek(), updates_iter.peek())
        {
            match existing_entry.doc_id.cmp(&update_entry.doc_id) {
                std::cmp::Ordering::Less => {
                    merged.push(*existing_entry);
                    existing_iter.next();
                }
                std::cmp::Ordering::Greater => {
                    merged.push(*update_entry);
                    updates_iter.next();
                }
                std::cmp::Ordering::Equal => {
                    merged.push(*update_entry);
                    existing_iter.next();
                    updates_iter.next();
                }
            }
        }

        merged.extend(existing_iter);
        merged.extend(updates_iter);
        *entries = merged;
    }

    /// Removes all posting entries for `point_id`.
    ///
    /// Returns `true` if the point had at least one entry in this segment.
    pub(super) fn delete(&mut self, point_id: u64) -> bool {
        self.doc_set.remove(&point_id);

        let mut any_removed = false;
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

        for term_id in &empty_terms {
            self.postings.remove(term_id);
            self.max_weights.remove(term_id);
        }

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
