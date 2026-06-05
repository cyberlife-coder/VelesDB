//! Concurrent edge store with sharded locking.
//!
//! This module provides `ConcurrentEdgeStore`, a thread-safe wrapper around
//! `EdgeStore` that uses sharding to reduce lock contention.
//!
//! Read-only queries and traversal are in `query.rs`.

// Reason: Numeric casts in edge store sharding are intentional:
// - u64->usize for node ID hashing: Node IDs are generated sequentially and fit in usize
// - Used for sharding only, actual storage uses u64 for persistence
#![allow(clippy::cast_possible_truncation)]

mod cascade;
mod persistence;
mod query;
mod snapshot;

use super::clustered_index::ClusteredIndex;
use super::csr_snapshot::{CsrSnapshot, SnapshotBuilder};
use super::edge::{EdgeStore, GraphEdge};
use super::label_table::LabelTable;
use crate::error::{Error, Result};
use arc_swap::ArcSwap;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};

/// Number of pending edge mutations that may accumulate before the lazy CSR
/// rebuild is forced (issue #905 debounce).
///
/// Below this threshold a dirty CSR snapshot is **not** rebuilt on read;
/// callers fall back to the always-correct per-shard traversal path instead
/// (see `traverse_bfs_config`). This amortises the O(N+E) clone-and-rebuild
/// across many writes rather than paying it on every single read that follows
/// a write. A pure write-count threshold (no wall-clock) keeps the behaviour
/// deterministic for single-threaded tests.
pub(crate) const CSR_REBUILD_WRITE_THRESHOLD: u64 = 64;

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
    /// Number of edge mutations accumulated since the last CSR rebuild.
    ///
    /// Used to debounce the lazy rebuild (issue #905): the next reader only
    /// pays for a full O(N+E) rebuild once this reaches
    /// [`CSR_REBUILD_WRITE_THRESHOLD`]. While dirty-but-below-threshold the
    /// CSR snapshot is intentionally stale and callers must consult the
    /// authoritative per-shard data (see [`Self::csr_is_authoritative`]).
    pending_writes: AtomicU64,
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
            pending_writes: AtomicU64::new(0),
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
            // Count every inserted edge toward the CSR rebuild debounce
            // (issue #905 follow-up). Reporting a flat `1` per batch would
            // keep a bulk-loaded graph permanently below the rebuild
            // threshold, so the CSR fast path would never engage.
            self.record_pending_writes(count as u64);
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
}

// Node-cascade cleanup helpers (collect_node_edges, gather_affected_shards,
// cleanup_shard_edges, deregister_edge_ids) are in cascade.rs
// Persistence (from_edge_store, save_to_file, load_from_file) is in persistence.rs
// CSR snapshot management (invalidate, rebuild, build) is in snapshot.rs

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
