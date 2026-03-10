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

mod query;

use super::edge::{EdgeStore, GraphEdge};
use crate::error::{Error, Result};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::HashSet;

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
}

impl ConcurrentEdgeStore {
    /// Creates a new concurrent edge store with the default number of shards.
    #[must_use]
    pub fn new() -> Self {
        Self::with_shards(DEFAULT_NUM_SHARDS)
    }

    /// Creates a new concurrent edge store with a specific number of shards.
    ///
    /// # Panics
    ///
    /// Panics if `num_shards` is 0 (would cause division-by-zero in shard_index).
    #[must_use]
    pub fn with_shards(num_shards: usize) -> Self {
        assert!(num_shards > 0, "num_shards must be at least 1");
        let shards = (0..num_shards)
            .map(|_| RwLock::new(EdgeStore::new()))
            .collect();
        Self {
            shards,
            num_shards,
            edge_ids: RwLock::new(FxHashMap::default()),
        }
    }

    /// Creates a concurrent edge store with optimal shard count for estimated edge count.
    ///
    /// **FLAG-6: Uses integer bit manipulation for ceiling log2.**
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
        Self::with_shards(optimal_shards)
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
        Ok(())
    }

    /// Removes an edge by ID using optimized 2-shard lookup.
    ///
    /// # Concurrency Safety
    ///
    /// Lock ordering: edge_ids FIRST, then shards in ascending order.
    pub fn remove_edge(&self, edge_id: u64) {
        let mut ids = self.edge_ids.write();

        let Some(&source_id) = ids.get(&edge_id) else {
            return;
        };

        let source_shard_idx = self.shard_index(source_id);
        let target_id = {
            let guard = self.shards[source_shard_idx].read();
            if let Some(edge) = guard.get_edge(edge_id) {
                edge.target()
            } else {
                ids.remove(&edge_id);
                return;
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
    }

    /// Removes all edges connected to a node (cascade delete, thread-safe).
    ///
    /// # Concurrency Safety
    ///
    /// Lock ordering: edge_ids FIRST, then shards in ascending order.
    pub fn remove_node_edges(&self, node_id: u64) {
        let mut ids = self.edge_ids.write();

        let node_shard = self.shard_index(node_id);

        // Phase 1: Collect all edges connected to this node (read-only)
        let (outgoing_edges, incoming_edges): (Vec<_>, Vec<_>) = {
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
        };

        // Phase 2: Collect all shards that need cleanup (BTreeSet = sorted ascending)
        let mut shards_to_clean: std::collections::BTreeSet<usize> =
            std::collections::BTreeSet::new();
        shards_to_clean.insert(node_shard);

        for (_, target) in &outgoing_edges {
            shards_to_clean.insert(self.shard_index(*target));
        }
        for (_, source) in &incoming_edges {
            shards_to_clean.insert(self.shard_index(*source));
        }

        // Phase 3: Acquire shard locks in ascending order and perform cleanup
        let mut guards: Vec<_> = shards_to_clean
            .iter()
            .map(|&idx| (idx, self.shards[idx].write()))
            .collect();

        // Phase 4: Clean up edges in all shards
        for (shard_idx, guard) in &mut guards {
            if *shard_idx == node_shard {
                guard.remove_node_edges(node_id);
            } else {
                for (edge_id, target) in &outgoing_edges {
                    if self.shard_index(*target) == *shard_idx {
                        guard.remove_edge_incoming_only(*edge_id);
                    }
                }
                for (edge_id, source) in &incoming_edges {
                    if self.shard_index(*source) == *shard_idx {
                        guard.remove_edge_outgoing_only(*edge_id);
                    }
                }
            }
        }

        // Phase 5: Remove edge IDs from global registry
        let mut removed: HashSet<u64> = HashSet::new();
        for (edge_id, _) in &outgoing_edges {
            if removed.insert(*edge_id) {
                ids.remove(edge_id);
            }
        }
        for (edge_id, _) in &incoming_edges {
            if removed.insert(*edge_id) {
                ids.remove(edge_id);
            }
        }
    }
}

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
