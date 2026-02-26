//! Read-only query and traversal methods for `ConcurrentEdgeStore`.
//!
//! Extracted from the main module for single-responsibility:
//! - Edge lookups (by node, by label, by ID)
//! - BFS traversal
//! - Edge count

use super::{ConcurrentEdgeStore, GraphEdge};
use std::collections::{HashSet, VecDeque};

impl ConcurrentEdgeStore {
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
