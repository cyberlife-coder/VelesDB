//! Concurrent edge storage with sharding for thread-safe graph operations.
//!
//! This module provides `ConcurrentEdgeStore`, a thread-safe wrapper around
//! `EdgeStore` that uses sharding to reduce lock contention.

use super::edge::{EdgeStore, GraphEdge};
use parking_lot::RwLock;
use std::collections::{HashSet, VecDeque};

/// Default number of shards for concurrent edge store.
const DEFAULT_NUM_SHARDS: usize = 64;

/// A thread-safe edge store using sharded locking.
///
/// Distributes edges across multiple shards based on source node ID
/// to reduce lock contention in multi-threaded scenarios.
///
/// # Lock Ordering
///
/// When acquiring multiple shard locks, always acquire in ascending
/// shard index order to prevent deadlocks.
#[repr(C, align(64))]
pub struct ConcurrentEdgeStore {
    shards: Vec<RwLock<EdgeStore>>,
    num_shards: usize,
}

impl ConcurrentEdgeStore {
    /// Creates a new concurrent edge store with the default number of shards.
    #[must_use]
    pub fn new() -> Self {
        Self::with_shards(DEFAULT_NUM_SHARDS)
    }

    /// Creates a new concurrent edge store with a specific number of shards.
    #[must_use]
    pub fn with_shards(num_shards: usize) -> Self {
        let shards = (0..num_shards)
            .map(|_| RwLock::new(EdgeStore::new()))
            .collect();
        Self { shards, num_shards }
    }

    /// Returns the shard index for a given node ID.
    #[inline]
    fn shard_index(&self, node_id: u64) -> usize {
        (node_id as usize) % self.num_shards
    }

    /// Adds an edge to the store (thread-safe).
    ///
    /// Acquires locks in ascending shard order to prevent deadlocks.
    pub fn add_edge(&self, edge: GraphEdge) {
        let source = edge.source();
        let target = edge.target();
        let shard_src = self.shard_index(source);
        let shard_tgt = self.shard_index(target);

        if shard_src == shard_tgt {
            let mut guard = self.shards[shard_src].write();
            guard.add_edge(edge);
        } else {
            // Acquire locks in ascending order to prevent deadlock
            let (first, second) = if shard_src < shard_tgt {
                (shard_src, shard_tgt)
            } else {
                (shard_tgt, shard_src)
            };

            let mut _guard1 = self.shards[first].write();
            let mut guard2 = self.shards[second].write();

            // Add to the source shard (where outgoing index is maintained)
            if first == shard_src {
                drop(_guard1);
                let mut src_guard = self.shards[shard_src].write();
                src_guard.add_edge(edge);
            } else {
                guard2.add_edge(edge);
            }
        }
    }

    /// Gets all outgoing edges from a node (thread-safe).
    #[must_use]
    pub fn get_outgoing(&self, node_id: u64) -> Vec<GraphEdge> {
        let shard = &self.shards[self.shard_index(node_id)];
        let guard = shard.read();
        guard.get_outgoing(node_id).into_iter().cloned().collect()
    }

    /// Gets all incoming edges to a node (thread-safe).
    #[must_use]
    pub fn get_incoming(&self, node_id: u64) -> Vec<GraphEdge> {
        let shard = &self.shards[self.shard_index(node_id)];
        let guard = shard.read();
        guard.get_incoming(node_id).into_iter().cloned().collect()
    }

    /// Gets neighbors (target nodes) of a given node.
    #[must_use]
    pub fn get_neighbors(&self, node_id: u64) -> Vec<u64> {
        self.get_outgoing(node_id)
            .iter()
            .map(GraphEdge::target)
            .collect()
    }

    /// Removes all edges connected to a node (cascade delete, thread-safe).
    pub fn remove_node_edges(&self, node_id: u64) {
        let shard = &self.shards[self.shard_index(node_id)];
        let mut guard = shard.write();
        guard.remove_node_edges(node_id);
    }

    /// Traverses the graph using BFS from a starting node.
    ///
    /// Returns all nodes reachable within `max_depth` hops.
    ///
    /// Uses Read-Copy-Drop pattern to avoid holding locks during traversal.
    #[must_use]
    pub fn traverse_bfs(&self, start: u64, max_depth: u32) -> Vec<u64> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((start, 0u32));

        while let Some((node, depth)) = queue.pop_front() {
            if depth > max_depth || !visited.insert(node) {
                continue;
            }

            // Read-Copy-Drop pattern: copy neighbors and drop guard immediately
            let neighbors: Vec<u64> = {
                let shard = &self.shards[self.shard_index(node)];
                let guard = shard.read();
                guard
                    .get_outgoing(node)
                    .iter()
                    .map(|e| e.target())
                    .collect()
            }; // Guard dropped here

            for neighbor in neighbors {
                if !visited.contains(&neighbor) {
                    queue.push_back((neighbor, depth + 1));
                }
            }
        }

        visited.into_iter().collect()
    }

    /// Returns the total edge count across all shards.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.shards.iter().map(|s| s.read().edge_count()).sum()
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
