//! Concurrent edge store with sharded locking.
//!
//! This module provides `ConcurrentEdgeStore`, a thread-safe wrapper around
//! `EdgeStore` that uses sharding to reduce lock contention.
//!
//! Read-only queries and traversal are in `query.rs`.

// SAFETY: Numeric casts in edge store sharding are intentional:
// - u64->usize for node ID hashing: Node IDs are generated sequentially and fit in usize
// - Used for sharding only, actual storage uses u64 for persistence
#![allow(clippy::cast_possible_truncation)]

mod persistence;
mod query;

use super::clustered_index::ClusteredIndex;
use super::csr_snapshot::{CsrSnapshot, SnapshotBuilder};
use super::edge::{EdgeStore, GraphEdge};
use super::label_table::LabelTable;
use crate::error::{Error, Result};
use arc_swap::ArcSwap;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Default number of shards for concurrent edge store.
/// Increased from 64 to 256 for better scalability with 10M+ edges (EPIC-019 US-001).
const DEFAULT_NUM_SHARDS: usize = 256;

/// Minimum edges per shard for efficiency.
/// Below this threshold, having more shards adds overhead without benefit.
const MIN_EDGES_PER_SHARD: usize = 1000;

/// Maximum recommended shards to limit memory overhead from RwLock + EdgeStore structures.
const MAX_SHARDS: usize = 512;

/// A thread-safe edge store using sharded locking.
///
/// Distributes edges across multiple shards based on source node ID
/// to reduce lock contention in multi-threaded scenarios.
///
/// # Cross-Shard Edge Storage Pattern
///
/// Edges that span different shards (source and target in different shards) are stored
/// in BOTH shards:
/// - **Source shard**: Full edge with outgoing + label indices (`add_edge`)
/// - **Target shard**: Edge copy with incoming index only (`add_edge_incoming_only`)
///
/// # Lock Ordering
///
/// When acquiring multiple shard locks, always acquire in ascending
/// shard index order to prevent deadlocks.
#[repr(C, align(64))]
pub struct ConcurrentEdgeStore {
    pub(super) shards: Vec<RwLock<EdgeStore>>,
    pub(super) num_shards: usize,
    /// Global registry of edge IDs with source node for optimized removal.
    /// Maps edge_id -> source_node_id for O(1) shard lookup during removal.
    /// F-19: FxHashMap ~2x faster than std HashMap for u64 keys (no SipHash).
    pub(super) edge_ids: RwLock<FxHashMap<u64, u64>>,
    /// CSR-like read snapshot for zero-copy neighbor lookups during BFS/DFS.
    ///
    /// Built on demand via [`build_read_snapshot()`](Self::build_read_snapshot).
    /// Invalidated to `None` on every write (`add_edge`, `remove_edge`,
    /// `remove_node_edges`). Read methods fall back to shard lookup when
    /// the snapshot is absent.
    clustered_snapshot: RwLock<Option<ClusteredIndex>>,
    /// Lock-free CSR snapshot for zero-copy reads via `ArcSwap`.
    ///
    /// Rebuilt lazily on the next read after a mutation sets `csr_dirty`.
    /// Readers load the current `Arc<CsrSnapshot>` without contention.
    csr_snapshot: ArcSwap<CsrSnapshot>,
    /// Dirty flag for lazy CSR snapshot rebuild.
    ///
    /// Set to `true` by every mutation (`add_edge`, `remove_edge`,
    /// `remove_node_edges`). The next read via `get_csr_snapshot()` or
    /// `traverse_bfs_csr()` rebuilds the snapshot and clears the flag.
    /// This eliminates O(N+E) rebuilds on every mutation, deferring the
    /// cost to the next read.
    csr_dirty: AtomicBool,
    /// Shared label table for interning edge labels during snapshot builds.
    label_table: RwLock<LabelTable>,
}

impl ConcurrentEdgeStore {
    /// Creates a new concurrent edge store with the default number of shards.
    ///
    /// Uses `DEFAULT_NUM_SHARDS` (compile-time constant > 0), so this
    /// constructor cannot fail in practice.
    #[allow(clippy::missing_panics_doc)] // Invariant: DEFAULT_NUM_SHARDS > 0
    #[must_use]
    pub fn new() -> Self {
        // DEFAULT_NUM_SHARDS is a compile-time constant > 0, so this cannot fail.
        Self::with_shards(DEFAULT_NUM_SHARDS).expect("invariant: DEFAULT_NUM_SHARDS > 0")
    }

    /// Creates a new concurrent edge store with a specific number of shards.
    ///
    /// # Errors
    ///
    /// Returns `Error::Config` if `num_shards` is 0 (would cause
    /// division-by-zero in shard_index).
    pub fn with_shards(num_shards: usize) -> crate::error::Result<Self> {
        if num_shards == 0 {
            return Err(crate::error::Error::Config(
                "num_shards must be at least 1".to_string(),
            ));
        }
        let shards = (0..num_shards)
            .map(|_| RwLock::new(EdgeStore::new()))
            .collect();
        Ok(Self {
            shards,
            num_shards,
            edge_ids: RwLock::new(FxHashMap::default()),
            clustered_snapshot: RwLock::new(None),
            csr_snapshot: ArcSwap::from_pointee(SnapshotBuilder::empty()),
            csr_dirty: AtomicBool::new(false),
            label_table: RwLock::new(LabelTable::new()),
        })
    }

    /// Creates a concurrent edge store with optimal shard count for estimated edge count.
    ///
    /// **FLAG-6: Uses integer bit manipulation for ceiling log2.**
    #[allow(clippy::missing_panics_doc)] // Invariant: optimal_shards >= 1 (clamped)
    #[must_use]
    pub fn with_estimated_edges(estimated_edges: usize) -> Self {
        let optimal_shards = if estimated_edges < MIN_EDGES_PER_SHARD {
            1
        } else {
            let target_shards = estimated_edges / MIN_EDGES_PER_SHARD;
            let power_of_2 = if target_shards <= 1 {
                0
            } else {
                usize::BITS - (target_shards - 1).leading_zeros()
            };
            (1usize << power_of_2).clamp(1, MAX_SHARDS)
        };
        // optimal_shards is always >= 1 (clamped above), so this cannot fail.
        Self::with_shards(optimal_shards).expect("invariant: optimal_shards >= 1")
    }

    /// Returns the shard index for a given node ID.
    #[inline]
    pub(super) fn shard_index(&self, node_id: u64) -> usize {
        (node_id as usize) % self.num_shards
    }

    /// Adds an edge to the store (thread-safe).
    ///
    /// Edges are stored in BOTH source and target shards:
    /// - Source shard: for outgoing index lookups
    /// - Target shard: for incoming index lookups
    ///
    /// When source and target are in different shards, locks are acquired
    /// in ascending shard index order to prevent deadlocks.
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    pub fn add_edge(&self, edge: GraphEdge) -> Result<()> {
        let edge_id = edge.id();

        {
            // CRITICAL: Hold edge_ids lock throughout the entire operation to prevent race
            // condition where remove_edge could free an ID while we're still inserting.
            // Lock ordering: edge_ids FIRST, then shards in ascending order.
            let mut ids = self.edge_ids.write();
            if ids.contains_key(&edge_id) {
                return Err(Error::EdgeExists(edge_id));
            }

            let source_id = edge.source();
            let source_shard = self.shard_index(source_id);
            let target_shard = self.shard_index(edge.target());

            if source_shard == target_shard {
                // Same shard: single lock, EdgeStore handles both indices
                let mut guard = self.shards[source_shard].write();
                guard.add_edge(edge)?;
                ids.insert(edge_id, source_id);
            } else {
                // Different shards: acquire locks in ascending order to prevent deadlock
                let (first_idx, second_idx) = if source_shard < target_shard {
                    (source_shard, target_shard)
                } else {
                    (target_shard, source_shard)
                };

                let mut first_guard = self.shards[first_idx].write();
                let mut second_guard = self.shards[second_idx].write();

                if source_shard < target_shard {
                    first_guard.add_edge_outgoing_only(edge.clone())?;
                    if let Err(e) = second_guard.add_edge_incoming_only(edge) {
                        first_guard.remove_edge_outgoing_only(edge_id);
                        return Err(e);
                    }
                } else {
                    second_guard.add_edge_outgoing_only(edge.clone())?;
                    if let Err(e) = first_guard.add_edge_incoming_only(edge) {
                        second_guard.remove_edge_outgoing_only(edge_id);
                        return Err(e);
                    }
                }
                ids.insert(edge_id, source_id);
            }
        } // All locks dropped here.
        self.invalidate_snapshot();
        self.rebuild_snapshot_best_effort();
        Ok(())
    }

    /// Adds multiple edges in batch with a single lock acquisition cycle.
    ///
    /// Acquires the `edge_ids` write lock once for the entire batch,
    /// inserts all edges into their respective shards, then invalidates
    /// the CSR snapshot once at the end. This is **10-50x faster** than
    /// calling `add_edge` in a loop for large batches.
    ///
    /// Edges that already exist (duplicate IDs) are silently skipped.
    ///
    /// # Returns
    ///
    /// Number of edges successfully added.
    pub fn add_edges_batch(&self, edges: Vec<GraphEdge>) -> usize {
        if edges.is_empty() {
            return 0;
        }

        let mut count = 0usize;
        {
            let mut ids = self.edge_ids.write();

            for edge in edges {
                let edge_id = edge.id();
                if ids.contains_key(&edge_id) {
                    continue;
                }

                let source_id = edge.source();
                let ok = self.insert_edge_into_shards(edge);

                if ok {
                    ids.insert(edge_id, source_id);
                    count += 1;
                }
            }
        }

        if count > 0 {
            self.invalidate_snapshot();
            self.rebuild_snapshot_best_effort();
        }
        count
    }

    /// Inserts a single edge into the correct shard(s), handling cross-shard locking.
    ///
    /// Returns `true` if the edge was successfully inserted.
    fn insert_edge_into_shards(&self, edge: GraphEdge) -> bool {
        let source_shard = self.shard_index(edge.source());
        let target_shard = self.shard_index(edge.target());

        if source_shard == target_shard {
            return self.shards[source_shard].write().add_edge(edge).is_ok();
        }

        // Cross-shard: acquire locks in ascending order to prevent deadlock.
        let (first_idx, second_idx) = if source_shard < target_shard {
            (source_shard, target_shard)
        } else {
            (target_shard, source_shard)
        };
        let mut first = self.shards[first_idx].write();
        let mut second = self.shards[second_idx].write();

        let (outgoing_guard, incoming_guard) = if source_shard < target_shard {
            (&mut first, &mut second)
        } else {
            (&mut second, &mut first)
        };

        let edge_id = edge.id();
        if outgoing_guard.add_edge_outgoing_only(edge.clone()).is_ok() {
            if incoming_guard.add_edge_incoming_only(edge).is_err() {
                outgoing_guard.remove_edge_outgoing_only(edge_id);
                return false;
            }
            true
        } else {
            false
        }
    }

    /// Removes an edge by ID using optimized 2-shard lookup.
    ///
    /// # Concurrency Safety
    ///
    /// Lock ordering: edge_ids FIRST, then shards in ascending order.
    pub fn remove_edge(&self, edge_id: u64) -> bool {
        {
            let mut ids = self.edge_ids.write();

            let Some(&source_id) = ids.get(&edge_id) else {
                return false;
            };

            let source_shard_idx = self.shard_index(source_id);
            let target_id = {
                let guard = self.shards[source_shard_idx].read();
                if let Some(edge) = guard.get_edge(edge_id) {
                    edge.target()
                } else {
                    ids.remove(&edge_id);
                    return false;
                }
            };

            let target_shard_idx = self.shard_index(target_id);

            if source_shard_idx == target_shard_idx {
                self.shards[source_shard_idx].write().remove_edge(edge_id);
            } else {
                let (first_idx, second_idx) = if source_shard_idx < target_shard_idx {
                    (source_shard_idx, target_shard_idx)
                } else {
                    (target_shard_idx, source_shard_idx)
                };
                let mut first = self.shards[first_idx].write();
                let mut second = self.shards[second_idx].write();

                if source_shard_idx < target_shard_idx {
                    first.remove_edge(edge_id);
                    second.remove_edge_incoming_only(edge_id);
                } else {
                    first.remove_edge_incoming_only(edge_id);
                    second.remove_edge(edge_id);
                }
            }

            ids.remove(&edge_id);
        } // All locks dropped here.
        self.invalidate_snapshot();
        self.rebuild_snapshot_best_effort();
        true
    }

    /// Removes all edges connected to a node (cascade delete, thread-safe).
    ///
    /// # Concurrency Safety
    ///
    /// Lock ordering: edge_ids FIRST, then shards in ascending order.
    pub fn remove_node_edges(&self, node_id: u64) {
        {
            let mut ids = self.edge_ids.write();
            let node_shard = self.shard_index(node_id);

            let (outgoing_edges, incoming_edges) = self.collect_node_edges(node_shard, node_id);

            let shards_to_clean =
                self.gather_affected_shards(node_shard, &outgoing_edges, &incoming_edges);

            let mut guards: Vec<_> = shards_to_clean
                .iter()
                .map(|&idx| (idx, self.shards[idx].write()))
                .collect();

            self.cleanup_shard_edges(
                &mut guards,
                node_shard,
                node_id,
                &outgoing_edges,
                &incoming_edges,
            );

            self.deregister_edge_ids(&mut ids, &outgoing_edges, &incoming_edges);
        }
        self.invalidate_snapshot();
        self.rebuild_snapshot_best_effort();
    }

    /// Collects all outgoing and incoming edges for a node (read-only).
    #[allow(clippy::type_complexity)] // Reason: tuple of (outgoing, incoming) edge lists is clear in context
    fn collect_node_edges(
        &self,
        node_shard: usize,
        node_id: u64,
    ) -> (Vec<(u64, u64)>, Vec<(u64, u64)>) {
        let guard = self.shards[node_shard].read();
        let outgoing: Vec<_> = guard
            .get_outgoing(node_id)
            .iter()
            .map(|e| (e.id(), e.target()))
            .collect();
        let incoming: Vec<_> = guard
            .get_incoming(node_id)
            .iter()
            .map(|e| (e.id(), e.source()))
            .collect();
        (outgoing, incoming)
    }

    /// Gathers the set of shard indices that need cleanup (sorted ascending for lock ordering).
    fn gather_affected_shards(
        &self,
        node_shard: usize,
        outgoing: &[(u64, u64)],
        incoming: &[(u64, u64)],
    ) -> std::collections::BTreeSet<usize> {
        let mut shards = std::collections::BTreeSet::new();
        shards.insert(node_shard);
        for (_, target) in outgoing {
            shards.insert(self.shard_index(*target));
        }
        for (_, source) in incoming {
            shards.insert(self.shard_index(*source));
        }
        shards
    }

    /// Cleans up edges in all affected shards.
    #[allow(clippy::too_many_arguments)]
    fn cleanup_shard_edges(
        &self,
        guards: &mut [(usize, parking_lot::RwLockWriteGuard<'_, super::EdgeStore>)],
        node_shard: usize,
        node_id: u64,
        outgoing: &[(u64, u64)],
        incoming: &[(u64, u64)],
    ) {
        for (shard_idx, guard) in guards {
            if *shard_idx == node_shard {
                guard.remove_node_edges(node_id);
            } else {
                for (edge_id, target) in outgoing {
                    if self.shard_index(*target) == *shard_idx {
                        guard.remove_edge_incoming_only(*edge_id);
                    }
                }
                for (edge_id, source) in incoming {
                    if self.shard_index(*source) == *shard_idx {
                        guard.remove_edge_outgoing_only(*edge_id);
                    }
                }
            }
        }
    }

    /// Removes edge IDs from the global registry, deduplicating.
    #[allow(clippy::unused_self)] // Reason: method on ConcurrentEdgeStore for API consistency
    fn deregister_edge_ids(
        &self,
        ids: &mut FxHashMap<u64, u64>,
        outgoing: &[(u64, u64)],
        incoming: &[(u64, u64)],
    ) {
        let mut removed: FxHashSet<u64> = FxHashSet::default();
        for (edge_id, _) in outgoing {
            if removed.insert(*edge_id) {
                ids.remove(edge_id);
            }
        }
        for (edge_id, _) in incoming {
            if removed.insert(*edge_id) {
                ids.remove(edge_id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CSR snapshot (read-only `ClusteredIndex` for zero-copy neighbor lookups)
// ---------------------------------------------------------------------------

impl ConcurrentEdgeStore {
    /// Invalidates the clustered read snapshot.
    ///
    /// Called by every write method so that stale data is never served.
    /// Readers fall back to per-shard lookup when the clustered snapshot
    /// is absent.
    #[inline]
    fn invalidate_snapshot(&self) {
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
    fn rebuild_snapshot_best_effort(&self) {
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

// Persistence (from_edge_store, save_to_file, load_from_file) is in persistence.rs

impl Default for ConcurrentEdgeStore {
    fn default() -> Self {
        Self::new()
    }
}

// Compile-time check: ConcurrentEdgeStore must be Send + Sync
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ConcurrentEdgeStore>();
};
