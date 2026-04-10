//! CSR snapshot management for `ConcurrentEdgeStore`.
//!
//! Extracted from `edge_concurrent/mod.rs` to reduce NLOC below the 500 threshold.
//! Contains: `invalidate_snapshot()`, `rebuild_snapshot_best_effort()`,
//! `rebuild_snapshot()`, `build_read_snapshot()`, `has_read_snapshot()`.

use super::super::clustered_index::ClusteredIndex;
use super::super::csr_snapshot::SnapshotBuilder;
use super::super::edge::EdgeStore;
use super::ConcurrentEdgeStore;
use crate::error::Result;
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl ConcurrentEdgeStore {
    /// Invalidates the clustered read snapshot.
    ///
    /// Called by every write method so that stale data is never served.
    /// Readers fall back to per-shard lookup when the clustered snapshot
    /// is absent.
    #[inline]
    pub(super) fn invalidate_snapshot(&self) {
        // Fast path: skip write lock if snapshot is already absent.
        let guard = self.clustered_snapshot.read();
        if guard.is_some() {
            drop(guard);
            *self.clustered_snapshot.write() = None;
        }
    }

    /// Best-effort CSR snapshot rebuild after a mutation.
    ///
    /// Sets the `csr_dirty` flag so the next read triggers a rebuild.
    /// This eliminates the O(N+E) rebuild on every `add_edge`/`remove_edge`,
    /// deferring the cost to the next read.
    #[inline]
    pub(super) fn rebuild_snapshot_best_effort(&self) {
        self.csr_dirty.store(true, Ordering::Release);
    }

    /// Rebuilds the lock-free `CsrSnapshot` from all shards.
    ///
    /// Acquires read locks on all shards sequentially, merges outgoing edges
    /// into a single `EdgeStore`, builds a `CsrSnapshot` via `SnapshotBuilder`,
    /// and swaps it atomically into `self.csr_snapshot`.
    ///
    /// On failure the previous snapshot is retained (readers see stale but
    /// structurally valid data).
    ///
    /// # Note
    ///
    /// This method does NOT acquire `edge_ids` — it reads directly from
    /// shard `EdgeStore`s. This avoids deadlock when called from mutation
    /// methods that already hold the `edge_ids` write lock.
    ///
    /// # Errors
    ///
    /// Returns `Error::SnapshotBuildFailed` if the merge or build fails.
    #[allow(clippy::unnecessary_wraps)] // Reason: Result kept for future allocation-failure propagation
    pub(crate) fn rebuild_snapshot(&self) -> Result<()> {
        // Build a merged EdgeStore from all shards (outgoing edges only).
        // We iterate shards directly instead of using `to_merged_edge_store()`
        // to avoid acquiring `edge_ids` (which may already be write-locked
        // by the calling mutation method).
        let mut merged = EdgeStore::new();
        for shard in &self.shards {
            let guard = shard.read();
            for edge in guard.all_edges() {
                // Ignore duplicates — cross-shard edges appear in both shards
                // but `add_edge` deduplicates by edge ID.
                let _ = merged.add_edge(edge.clone());
            }
        }
        let label_table = self.label_table.read();
        let new_snapshot = SnapshotBuilder::build(&merged, &label_table);
        self.csr_snapshot.store(Arc::new(new_snapshot));
        Ok(())
    }

    /// Builds a CSR-like read snapshot from current shard state.
    ///
    /// The snapshot stores only outgoing neighbor **target node IDs** per source
    /// node in contiguous memory, enabling [`with_neighbors()`](Self::with_neighbors)
    /// to provide zero-copy `&[u64]` access without shard locking.
    ///
    /// # Limitation — target IDs only
    ///
    /// The snapshot does **not** store edge IDs, labels, or properties.
    /// It is optimized for BFS neighbor expansion where only connectivity
    /// matters. To retrieve full edge metadata (edge ID, label, properties),
    /// use [`get_outgoing()`](Self::get_outgoing) which reads from the
    /// authoritative shard data.
    ///
    /// Call this after bulk inserts, after `flush()`, or after loading
    /// from disk. The snapshot is automatically invalidated on any write.
    pub fn build_read_snapshot(&self) {
        let ids = self.edge_ids.read();
        let edge_count = ids.len();
        // Rough estimate: each edge contributes one outgoing target entry.
        let mut snapshot = ClusteredIndex::with_capacity(edge_count, edge_count);

        for (&edge_id, &source_id) in ids.iter() {
            let shard_idx = self.shard_index(source_id);
            let guard = self.shards[shard_idx].read();
            if let Some(edge) = guard.get_edge(edge_id) {
                snapshot.insert(source_id, edge.target());
            }
        }

        // Compact once to eliminate any fragmentation from insert order.
        snapshot.compact();

        *self.clustered_snapshot.write() = Some(snapshot);

        // Also rebuild the lock-free CSR snapshot.
        let _ = self.rebuild_snapshot();
    }

    /// Returns `true` if a CSR read snapshot is currently available.
    #[must_use]
    pub fn has_read_snapshot(&self) -> bool {
        self.clustered_snapshot.read().is_some()
    }
}
