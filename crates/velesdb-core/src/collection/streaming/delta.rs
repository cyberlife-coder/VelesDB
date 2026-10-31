//! The write buffer behind the deferred indexer.
//!
//! A [`DeltaBuffer`] holds vectors written but not yet inserted into the HNSW
//! graph, one entry per id (a second write of an id replaces the first). The
//! deferred indexer (`deferred.rs`) owns one: every search reads its ids
//! before the index search and scores those points on what storage holds for
//! them (`collection/search/pending.rs`), so they are found before a merge
//! indexes them, and a merge retires an entry only once it has indexed it.
//!
//! # Lock ordering
//!
//! A `DeltaBuffer`'s lock ranks with its owner's: the deferred indexer's, at
//! position **11**. Code must never hold it while acquiring a lower-numbered
//! lock.

use parking_lot::RwLock;
use std::collections::HashSet;

/// Vectors written but not yet in the HNSW index, one entry per id.
pub struct DeltaBuffer {
    /// Buffered `(point_id, vector)` pairs awaiting index insertion.
    points: RwLock<Vec<(u64, Vec<f32>)>>,
}

impl DeltaBuffer {
    /// Creates an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            points: RwLock::new(Vec::new()),
        }
    }

    /// Buffers `vector` for `id`, replacing an entry the id already has.
    ///
    /// The retain-then-push is O(n) but acceptable: the buffer is bounded
    /// by `merge_threshold` (typically 1024-4096 entries).
    pub fn push(&self, id: u64, vector: Vec<f32>) {
        let mut points = self.points.write();
        points.retain(|(existing_id, _)| *existing_id != id);
        points.push((id, vector));
    }

    /// Buffers every entry, each replacing an entry its id already has.
    ///
    /// # Lock scope (PERF3)
    ///
    /// The iterator is materialized **before** taking the write lock: callers
    /// pass lazy iterators whose items perform `O(N×D)` vector copies
    /// (e.g. `to_vec()` in `bulk_index_or_defer`), and running those copies
    /// under the write lock would stall every concurrent search reading the
    /// buffer's ids for the duration of the batch copy.
    pub fn extend(&self, entries: impl IntoIterator<Item = (u64, Vec<f32>)>) {
        let new_entries: Vec<(u64, Vec<f32>)> = entries.into_iter().collect();
        if new_entries.is_empty() {
            return;
        }
        let new_ids: HashSet<u64> = new_entries.iter().map(|(id, _)| *id).collect();
        let mut points = self.points.write();
        points.retain(|(existing_id, _)| !new_ids.contains(existing_id));
        points.extend(new_entries);
    }

    /// Removes the entry of `id`: the point was deleted.
    pub fn remove(&self, id: u64) {
        self.points.write().retain(|(eid, _)| *eid != id);
    }

    /// The number of buffered entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.read().len()
    }

    /// Whether the buffer holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The ids it holds, each once, sorted.
    #[must_use]
    pub(crate) fn ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.points.read().iter().map(|&(id, _)| id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// A copy of the `(id, vector)` pairs it holds.
    #[must_use]
    pub(crate) fn entries(&self) -> Vec<(u64, Vec<f32>)> {
        self.points.read().clone()
    }

    /// Removes the entries `indexed` names with the same vector: what a merge
    /// has indexed. An entry written again since keeps its newer vector and
    /// stays for the next merge.
    pub(crate) fn retire(&self, indexed: &[(u64, &[f32])]) {
        if indexed.is_empty() {
            return;
        }
        let done: rustc_hash::FxHashMap<u64, &[f32]> = indexed.iter().copied().collect();
        self.points
            .write()
            .retain(|(id, v)| done.get(id).is_none_or(|indexed| *indexed != v.as_slice()));
    }
}

impl Default for DeltaBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "delta_unit_tests.rs"]
mod tests;
