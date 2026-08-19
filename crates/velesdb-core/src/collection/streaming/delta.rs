//! Delta buffer for accumulating vectors during HNSW rebuilds.
//!
//! The [`DeltaBuffer`] holds recently inserted vectors that have not yet been
//! indexed into the HNSW graph (e.g., because a rebuild is in progress).
//! The search pipeline brute-force scans this buffer and merges results with
//! HNSW results for immediate searchability via
//! [`super::delta_merge::merge_with_delta`].
//!
//! # State machine
//!
//! The buffer transitions through three states encoded in the internal `state` field:
//!
//! ```text
//! INACTIVE (0) --activate()--> ACTIVE (1) --deactivate_and_drain()--> DRAINING (2) --> INACTIVE (0)
//! ```
//!
//! - `push` / `extend`: only write when `ACTIVE`.
//! - `search`: scan when `ACTIVE` or `DRAINING` (so concurrent searches during
//!   drain still see the buffered vectors).
//!
//! [`DeltaBuffer::activate`] is an unconditional store kept for the idempotent
//! callers in the rebuild path. [`DeltaBuffer::try_activate`] is the hardened
//! variant: it uses `compare_exchange(INACTIVE, ACTIVE)` and returns
//! [`ActivateError::AlreadyActive`] on re-entrance, so a double-activation bug
//! (two rebuilds racing on the same buffer) surfaces instead of being silently
//! swallowed (STREAM-9).
//!
//! # Lock ordering
//!
//! `DeltaBuffer` is at position **10** in the collection lock order
//! (after `sparse_indexes` at 9). Code must never hold a delta buffer lock
//! while acquiring a lower-numbered lock.

use crate::distance::DistanceMetric;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};

/// Buffer is inactive — not accumulating writes.
const INACTIVE: u8 = 0;
/// Buffer is actively accumulating writes (HNSW rebuild in progress).
const ACTIVE: u8 = 1;
/// Buffer is draining — no new writes accepted, but still readable for search.
const DRAINING: u8 = 2;

/// Error returned by [`DeltaBuffer::try_activate`] when activation is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ActivateError {
    /// The buffer was not `INACTIVE` (it is `ACTIVE` or `DRAINING`), so a
    /// concurrent activation/drain already owns it. Signals a double-activation
    /// bug rather than a recoverable condition.
    #[error("delta buffer is already active or draining; double-activation rejected")]
    AlreadyActive,
}

/// Delta buffer for streaming inserts during HNSW rebuilds.
///
/// Accumulates `(point_id, vector)` pairs that are in storage but not yet in
/// the HNSW index. When active, search methods brute-force scan the buffer
/// and merge results with HNSW results via
/// [`super::delta_merge::merge_with_delta`].
pub struct DeltaBuffer {
    /// Buffered `(point_id, vector)` pairs awaiting index insertion.
    points: RwLock<Vec<(u64, Vec<f32>)>>,

    /// State machine: `INACTIVE` | `ACTIVE` | `DRAINING`.
    state: AtomicU8,
}

impl DeltaBuffer {
    /// Creates an empty, inactive delta buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            points: RwLock::new(Vec::new()),
            state: AtomicU8::new(INACTIVE),
        }
    }

    /// Returns `true` if the delta buffer is actively accumulating vectors
    /// (i.e., an HNSW rebuild is in progress).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state.load(Ordering::Acquire) == ACTIVE
    }

    /// Returns true if the buffer contains data that should be merged into search results.
    ///
    /// This is true in both `ACTIVE` and `DRAINING` states: the buffer holds
    /// vectors not yet present in HNSW, so searches must include them regardless
    /// of whether new writes are still being accepted.
    #[must_use]
    pub fn is_searchable(&self) -> bool {
        let s = self.state.load(Ordering::Acquire);
        s == ACTIVE || s == DRAINING
    }

    /// Activates the delta buffer (marks a rebuild as in progress).
    ///
    /// While active, the drain loop will push vectors into this buffer so
    /// that search can find them before they are indexed into HNSW.
    ///
    /// Idempotent: calling `activate()` when already active is a no-op.
    pub fn activate(&self) {
        self.state.store(ACTIVE, Ordering::Release);
    }

    /// Activates the buffer via compare-and-swap, rejecting double-activation.
    ///
    /// Transitions `INACTIVE → ACTIVE` atomically. Unlike [`activate`](Self::activate),
    /// this surfaces a re-entrant activation: if the buffer is already `ACTIVE`
    /// or `DRAINING` (i.e., not `INACTIVE`), it returns
    /// [`ActivateError::AlreadyActive`] instead of silently overwriting the
    /// state. Use this on the rebuild entry path so two concurrent rebuilds on
    /// the same collection cannot both believe they own the buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ActivateError::AlreadyActive`] when the buffer is not `INACTIVE`.
    pub fn try_activate(&self) -> Result<(), ActivateError> {
        self.state
            .compare_exchange(INACTIVE, ACTIVE, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| ActivateError::AlreadyActive)
    }

    /// Deactivates the buffer and drains all buffered points.
    ///
    /// Transitions `ACTIVE → DRAINING`, takes the points, then sets
    /// `INACTIVE`. Any concurrent `search` call that observes `DRAINING`
    /// may race with this method and observe an empty buffer — that is
    /// architecturally acceptable. The real searchable-immediately guarantee
    /// is provided by the HNSW index rebuild completing after drain
    /// incorporates all drained vectors. Searches racing with
    /// `deactivate_and_drain` during the DRAINING window may miss these
    /// vectors transiently; they will be found via HNSW once the rebuild
    /// completes.
    ///
    /// Returns the accumulated `(point_id, vector)` pairs for progressive
    /// merge into the newly rebuilt HNSW index. After this call, the buffer
    /// is empty and inactive.
    pub fn deactivate_and_drain(&self) -> Vec<(u64, Vec<f32>)> {
        // Mark as DRAINING so concurrent searches can still observe the buffer
        // while we hold the write lock.
        self.state.store(DRAINING, Ordering::Release);
        let mut points = self.points.write();
        let drained = std::mem::take(&mut *points);
        // Set INACTIVE before dropping write lock: this ensures no observable window
        // where state == DRAINING but buffer is empty. A concurrent activate() call
        // seeing INACTIVE will store ACTIVE, and any subsequent push() will contend
        // for the write lock (still held here) then see the empty-but-active buffer.
        // This is correct: the activate→push sequence works on a clean buffer.
        self.state.store(INACTIVE, Ordering::Release);
        drop(points);
        drained
    }

    /// Pushes a single entry into the delta buffer (upsert semantics).
    ///
    /// If an entry with the same `id` already exists, it is replaced.
    /// This prevents duplicate IDs from accumulating when the same point
    /// is inserted multiple times during an HNSW rebuild.
    ///
    /// The retain-then-push is O(n) but acceptable: the buffer is bounded
    /// by `merge_threshold` (typically 1024-4096 entries).
    ///
    /// No-op if the buffer is not in `ACTIVE` state. The check is performed
    /// **inside** the write lock to close the TOCTOU window between `is_active()`
    /// and the actual write.
    pub fn push(&self, id: u64, vector: Vec<f32>) {
        let mut points = self.points.write();
        if self.state.load(Ordering::Acquire) == ACTIVE {
            points.retain(|(existing_id, _)| *existing_id != id);
            points.push((id, vector));
        }
    }

    /// Extends the delta buffer with multiple entries (upsert semantics).
    ///
    /// For each entry, any existing entry with the same ID is replaced.
    /// This prevents duplicate IDs from accumulating in the buffer.
    ///
    /// No-op if the buffer is not in `ACTIVE` state. The check is performed
    /// **inside** the write lock to close the TOCTOU window between `is_active()`
    /// and the actual write.
    ///
    /// # Lock scope (PERF3)
    ///
    /// The iterator is materialized **before** taking the write lock: callers
    /// pass lazy iterators whose items perform `O(N×D)` vector copies
    /// (e.g. `to_vec()` in `bulk_index_or_defer`), and running those copies
    /// under the write lock would stall every concurrent brute-force search
    /// (read lock) for the duration of the batch copy. If the buffer turns
    /// out to be inactive the materialized entries are dropped — a rare,
    /// benign waste (the deferred path activates the buffer just before
    /// extending).
    pub fn extend(&self, entries: impl IntoIterator<Item = (u64, Vec<f32>)>) {
        let new_entries: Vec<(u64, Vec<f32>)> = entries.into_iter().collect();
        if new_entries.is_empty() {
            return;
        }
        let new_ids: HashSet<u64> = new_entries.iter().map(|(id, _)| *id).collect();
        let mut points = self.points.write();
        if self.state.load(Ordering::Acquire) == ACTIVE {
            points.retain(|(existing_id, _)| !new_ids.contains(existing_id));
            points.extend(new_entries);
        }
    }

    /// Removes all entries matching the given point ID from the buffer.
    ///
    /// Works in any state (`ACTIVE`, `DRAINING`, or `INACTIVE`): a delete
    /// must always purge stale data regardless of the buffer lifecycle.
    /// This prevents ghost results where a deleted vector is still returned
    /// by the delta brute-force scan.
    pub fn remove(&self, id: u64) {
        self.points.write().retain(|(eid, _)| *eid != id);
    }

    /// Returns the number of buffered entries.
    ///
    /// Takes a single read lock. Use [`stats`](Self::stats) when both `len`
    /// and `is_empty` are needed to avoid two separate lock acquisitions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.read().len()
    }

    /// Returns `true` if the buffer contains no entries.
    ///
    /// Delegates to `len() == 0` (single lock acquisition).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `(len, is_empty)` under a single read lock.
    ///
    /// Prefer this over calling `len()` and `is_empty()` separately when both
    /// values are needed, to avoid acquiring the read lock twice.
    #[must_use]
    pub fn stats(&self) -> (usize, bool) {
        let len = self.points.read().len();
        (len, len == 0)
    }

    /// Brute-force searches the delta buffer for the k nearest neighbors.
    ///
    /// Returns an empty `Vec` if the buffer is neither `ACTIVE` nor `DRAINING`.
    /// Computes distances directly under a single read lock — without cloning
    /// the buffer — and only materializes the compact `(id, score)` result Vec
    /// (`O(M)`), never a full `O(M×D)` copy of the vector data. The distance
    /// scan is `O(M×D)` regardless, so the lock is held for the same order of
    /// work as the previous snapshot copy but does useful computation instead
    /// of allocating and copying. The (potentially larger) sort runs after the
    /// lock is released.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, metric: DistanceMetric) -> Vec<(u64, f32)> {
        let current_state = self.state.load(Ordering::Acquire);
        if current_state != ACTIVE && current_state != DRAINING {
            return Vec::new();
        }

        // Compute distances in place under the read lock; only the compact
        // result Vec escapes it. Sorting/truncation happen after release.
        let mut results: Vec<(u64, f32)> = {
            let points = self.points.read();
            if points.is_empty() {
                return Vec::new();
            }
            points
                .iter()
                .map(|(id, vec)| (*id, metric.calculate(query, vec)))
                .collect()
        };

        metric.sort_results(&mut results);
        results.truncate(k);
        results
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
