//! Concurrent edge storage with sharding for thread-safe graph operations.
//!
//! This module provides `ConcurrentEdgeStore`, a thread-safe wrapper around
//! `EdgeStore` that uses sharding to reduce lock contention.

// SAFETY: Numeric casts in edge store sharding are intentional:
// - u64->usize for node ID hashing: Node IDs are generated sequentially and fit in usize
// - Used for sharding only, actual storage uses u64 for persistence
#![allow(clippy::cast_possible_truncation)]

use super::edge::{EdgeStore, GraphEdge};
use crate::error::{Error, Result};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};

/// Default number of shards for concurrent edge store.
/// Increased from 64 to 256 for better scalability with 10M+ edges (EPIC-019 US-001).
/// With 256 shards and 10M edges, each shard contains ~39K edges (vs ~156K with 64 shards),
/// reducing lock contention in multi-threaded scenarios.
///
/// For smaller graphs, use `with_estimated_edges()` to get optimal shard count.
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
/// This enables O(1) lookups for both `get_outgoing(node)` and `get_incoming(node)`
/// without cross-shard queries. Label indices are maintained only in the source shard
/// to avoid duplication.
///
/// # Lock Ordering
///
/// When acquiring multiple shard locks, always acquire in ascending
/// shard index order to prevent deadlocks.
#[repr(C, align(64))]
pub struct ConcurrentEdgeStore {
    shards: Vec<RwLock<EdgeStore>>,
    num_shards: usize,
    /// Global registry of edge IDs with source node for optimized removal.
    /// Maps edge_id -> source_node_id for O(1) shard lookup during removal.
    ///
    /// **FLAG-5: Changed from HashSet to HashMap intentionally.**
    /// This trades ~8 bytes extra memory per edge for O(1) removal lookup
    /// instead of O(shards) iteration. Worth it for graphs with frequent deletions.
    edge_ids: RwLock<std::collections::HashMap<u64, u64>>,
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
            edge_ids: RwLock::new(HashMap::new()),
        }
    }

    /// Creates a concurrent edge store with optimal shard count for estimated edge count.
    ///
    /// # Shard Count Selection
    ///
    /// - `< 1K edges`: 1 shard (no sharding overhead)
    /// - `1K - 64K edges`: 16-64 shards
    /// - `64K - 1M edges`: 64-128 shards  
    /// - `> 1M edges`: 256 shards (default)
    ///
    /// This avoids memory overhead of 256 shards for small graphs while
    /// maintaining good concurrency for large graphs.
    ///
    /// **FLAG-6: Uses integer bit manipulation for ceiling log2.**
    /// Formula: `ceil(log2(n)) = bits - leading_zeros(n-1)` for n > 1.
    /// This avoids floating-point imprecision from `(n as f64).log2().ceil()`.
    #[must_use]
    pub fn with_estimated_edges(estimated_edges: usize) -> Self {
        let optimal_shards = if estimated_edges < MIN_EDGES_PER_SHARD {
            1
        } else {
            let target_shards = estimated_edges / MIN_EDGES_PER_SHARD;
            // Use integer math for log2 ceiling: avoid floating-point imprecision
            // Formula: ceil(log2(n)) = bits - leading_zeros(n-1) for n > 1
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
    fn shard_index(&self, node_id: u64) -> usize {
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

        // Note: EdgeStore's duplicate check is now redundant but kept for safety
        if source_shard == target_shard {
            // Same shard: single lock, EdgeStore handles both indices
            let mut guard = self.shards[source_shard].write();
            guard.add_edge(edge)?;
            // Store edge_id -> source_id for optimized removal
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

            // Add to source shard (outgoing index)
            // Add to target shard (incoming index)
            // Handle errors with proper rollback
            if source_shard < target_shard {
                // first = source, second = target
                first_guard.add_edge_outgoing_only(edge.clone())?;
                if let Err(e) = second_guard.add_edge_incoming_only(edge) {
                    // Rollback first shard operation
                    first_guard.remove_edge_outgoing_only(edge_id);
                    return Err(e);
                }
            } else {
                // first = target, second = source
                second_guard.add_edge_outgoing_only(edge.clone())?;
                if let Err(e) = first_guard.add_edge_incoming_only(edge) {
                    // Rollback second shard operation
                    second_guard.remove_edge_outgoing_only(edge_id);
                    return Err(e);
                }
            }
            // Insert AFTER successful shard mutations - store source for optimized removal
            ids.insert(edge_id, source_id);
        }
        Ok(())
    }

    /// Removes an edge by ID using optimized 2-shard lookup.
    ///
    /// # Performance
    ///
    /// Instead of iterating all 256 shards, uses stored source_id to find
    /// the exact 2 shards (source + target) where the edge is stored.
    ///
    /// # Concurrency Safety
    ///
    /// Lock ordering: edge_ids FIRST, then shards in ascending order.
    pub fn remove_edge(&self, edge_id: u64) {
        // Acquire edge_ids lock FIRST (same ordering as add_edge)
        let mut ids = self.edge_ids.write();

        // Get source_id for optimized shard lookup
        let source_id = match ids.get(&edge_id) {
            Some(&src) => src,
            None => return, // Edge doesn't exist
        };

        // Get the edge to find target_id (need to read from source shard)
        let source_shard_idx = self.shard_index(source_id);
        let target_id = {
            let guard = self.shards[source_shard_idx].read();
            match guard.get_edge(edge_id) {
                Some(edge) => edge.target(),
                None => {
                    // Edge in registry but not in shard - cleanup registry
                    ids.remove(&edge_id);
                    return;
                }
            }
        };

        let target_shard_idx = self.shard_index(target_id);

        // Remove from only the relevant shards (2 max instead of 256)
        if source_shard_idx == target_shard_idx {
            // Same shard: full removal from single shard
            self.shards[source_shard_idx].write().remove_edge(edge_id);
        } else {
            // Cross-shard: use specialized methods for efficiency
            // Source shard has outgoing + label indices, target shard has incoming only
            // Acquire locks in ascending order to prevent deadlock
            let (first_idx, second_idx) = if source_shard_idx < target_shard_idx {
                (source_shard_idx, target_shard_idx)
            } else {
                (target_shard_idx, source_shard_idx)
            };
            let mut first = self.shards[first_idx].write();
            let mut second = self.shards[second_idx].write();

            // Use specialized removal: source gets full removal, target gets incoming-only
            if source_shard_idx < target_shard_idx {
                first.remove_edge(edge_id); // Source: full cleanup
                second.remove_edge_incoming_only(edge_id); // Target: incoming index only
            } else {
                first.remove_edge_incoming_only(edge_id); // Target: incoming index only
                second.remove_edge(edge_id); // Source: full cleanup
            }
        }

        // Remove from global registry
        ids.remove(&edge_id);
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

    /// Gets outgoing edges filtered by label (thread-safe).
    ///
    /// # Performance Note
    ///
    /// This method delegates to the underlying `EdgeStore::get_outgoing_by_label`
    /// which uses the composite index `(source_id, label) -> edge_ids` for O(1) lookup
    /// when available (EPIC-019 US-003). Falls back to filtering if index not populated.
    #[must_use]
    pub fn get_outgoing_by_label(&self, node_id: u64, label: &str) -> Vec<GraphEdge> {
        let shard_idx = self.shard_index(node_id);
        let shard = self.shards[shard_idx].read();
        shard
            .get_outgoing_by_label(node_id, label)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Gets incoming edges filtered by label (thread-safe).
    #[must_use]
    pub fn get_incoming_by_label(&self, node_id: u64, label: &str) -> Vec<GraphEdge> {
        self.get_incoming(node_id)
            .into_iter()
            .filter(|e| e.label() == label)
            .collect()
    }

    /// Gets all edges with a specific label across all shards.
    ///
    /// # Performance Warning
    ///
    /// This method iterates through ALL shards and aggregates results.
    /// For large graphs with many shards, this can be expensive.
    /// Consider using `get_outgoing_by_label(node_id, label)` if you know
    /// the source node, which is O(k) instead of O(shards × edges_per_label).
    ///
    /// # Use Cases
    ///
    /// - Schema introspection (listing all edges of a type)
    /// - Batch operations on all edges of a type
    /// - Testing and debugging
    #[must_use]
    pub fn get_edges_by_label(&self, label: &str) -> Vec<GraphEdge> {
        self.shards
            .iter()
            .flat_map(|shard| {
                shard
                    .read()
                    .get_edges_by_label(label)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Checks if an edge with the given ID exists.
    #[must_use]
    pub fn contains_edge(&self, edge_id: u64) -> bool {
        self.edge_ids.read().contains_key(&edge_id)
    }

    /// Gets an edge by ID using optimized source shard lookup.
    ///
    /// Returns `None` if the edge doesn't exist.
    #[must_use]
    pub fn get_edge(&self, edge_id: u64) -> Option<GraphEdge> {
        // Get source_id from registry for direct shard lookup
        let source_id = *self.edge_ids.read().get(&edge_id)?;
        let shard_idx = self.shard_index(source_id);
        self.shards[shard_idx].read().get_edge(edge_id).cloned()
    }

    /// Removes all edges connected to a node (cascade delete, thread-safe).
    ///
    /// Handles cross-shard cleanup: collects all edges, then removes from all
    /// relevant shards with proper lock ordering to prevent deadlocks.
    ///
    /// # Concurrency Safety
    ///
    /// Lock ordering: edge_ids FIRST, then shards in ascending order.
    /// This matches add_edge/remove_edge ordering to prevent deadlocks.
    /// The edge_ids lock is held throughout to prevent add_edge from
    /// inserting IDs we're about to remove.
    pub fn remove_node_edges(&self, node_id: u64) {
        // CRITICAL: Acquire edge_ids lock FIRST (same ordering as add_edge/remove_edge)
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

        // Phase 2: Collect all shards that need cleanup
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
        // BTreeSet iteration is already sorted ascending
        let mut guards: Vec<_> = shards_to_clean
            .iter()
            .map(|&idx| (idx, self.shards[idx].write()))
            .collect();

        // Phase 4: Clean up edges in all shards
        for (shard_idx, guard) in &mut guards {
            if *shard_idx == node_shard {
                // Main shard: full cleanup
                guard.remove_node_edges(node_id);
            } else {
                // Other shards: clean only the cross-shard edge entries
                for (edge_id, target) in &outgoing_edges {
                    if self.shard_index(*target) == *shard_idx {
                        // This edge's incoming index is in this shard
                        guard.remove_edge_incoming_only(*edge_id);
                    }
                }
                for (edge_id, source) in &incoming_edges {
                    if self.shard_index(*source) == *shard_idx {
                        // This edge's outgoing index is in this shard
                        guard.remove_edge_outgoing_only(*edge_id);
                    }
                }
            }
        }

        // Phase 5: Remove edge IDs from global registry (still holding lock)
        // Note: Use a set to deduplicate IDs (self-loops appear in both lists)
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
    ///
    /// Uses outgoing edge count to avoid double-counting edges that span shards.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.shards
            .iter()
            .map(|s| s.read().outgoing_edge_count())
            .sum()
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
