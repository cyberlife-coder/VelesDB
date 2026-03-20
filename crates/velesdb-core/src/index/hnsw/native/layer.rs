//! HNSW Layer implementation.
//!
//! A single layer in the HNSW hierarchy containing node adjacency lists.

use parking_lot::RwLock;

/// Unique identifier for a node in the graph.
pub type NodeId = usize;

/// A single layer in the HNSW hierarchy.
#[derive(Debug)]
pub struct Layer {
    /// Adjacency list: node_id -> list of neighbor node_ids
    pub(crate) neighbors: Vec<RwLock<Vec<NodeId>>>,
}

impl Layer {
    /// Creates a new layer with the given capacity.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            neighbors: (0..capacity).map(|_| RwLock::new(Vec::new())).collect(),
        }
    }

    /// Ensures the layer has capacity for the given node_id.
    pub(crate) fn ensure_capacity(&mut self, node_id: NodeId) {
        while self.neighbors.len() <= node_id {
            self.neighbors.push(RwLock::new(Vec::new()));
        }
    }

    /// Gets the neighbors of a node.
    #[inline]
    pub(crate) fn get_neighbors(&self, node_id: NodeId) -> Vec<NodeId> {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].read().clone()
        } else {
            Vec::new()
        }
    }

    /// Runs a closure with immutable access to a node's adjacency list under a read lock.
    #[inline]
    pub(crate) fn with_neighbors<R>(
        &self,
        node_id: NodeId,
        f: impl FnOnce(&[NodeId]) -> R,
    ) -> Option<R> {
        if node_id < self.neighbors.len() {
            let guard = self.neighbors[node_id].read();
            Some(f(&guard))
        } else {
            None
        }
    }

    /// Sets the neighbors for a node.
    #[inline]
    pub(crate) fn set_neighbors(&self, node_id: NodeId, neighbors: Vec<NodeId>) {
        if node_id < self.neighbors.len() {
            *self.neighbors[node_id].write() = neighbors;
        }
    }

    /// Mutates the neighbors for a node in-place under a single write lock.
    #[inline]
    pub(crate) fn with_neighbors_mut<R>(
        &self,
        node_id: NodeId,
        f: impl FnOnce(&mut Vec<NodeId>) -> R,
    ) -> Option<R> {
        if node_id < self.neighbors.len() {
            let mut guard = self.neighbors[node_id].write();
            Some(f(&mut guard))
        } else {
            None
        }
    }

    /// Adds a neighbor to a node's adjacency list.
    #[allow(dead_code)]
    pub(super) fn add_neighbor(&self, node_id: NodeId, neighbor: NodeId) {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].write().push(neighbor);
        }
    }

    /// Remaps all neighbor IDs using the provided old-to-new mapping.
    ///
    /// After graph reordering, node IDs change. This method updates every
    /// neighbor reference in the layer to use the new IDs, and reorders the
    /// adjacency lists themselves so that slot `new_id` contains the neighbors
    /// of the node that was formerly at `old_id`.
    pub(crate) fn remap_ids(&mut self, old_to_new: &[usize]) {
        let count = old_to_new.len();

        // Phase 1: Remap neighbor IDs within each adjacency list.
        for lock in &self.neighbors {
            let mut neighbors = lock.write();
            for id in neighbors.iter_mut() {
                if *id < count {
                    *id = old_to_new[*id];
                }
            }
        }

        // Phase 2: Reorder the adjacency lists themselves.
        // Extract all lists, then place each at its new position.
        let mut extracted: Vec<Vec<NodeId>> = self
            .neighbors
            .iter()
            .map(|lock| std::mem::take(&mut *lock.write()))
            .collect();

        // Build reordered lists: new slot `old_to_new[old]` gets `extracted[old]`
        let mut reordered: Vec<Vec<NodeId>> = vec![Vec::new(); extracted.len()];
        for (old_id, list) in extracted.drain(..).enumerate() {
            if old_id < count {
                reordered[old_to_new[old_id]] = list;
            }
        }

        // Write back
        for (i, lock) in self.neighbors.iter().enumerate() {
            if i < reordered.len() {
                *lock.write() = std::mem::take(&mut reordered[i]);
            }
        }
    }
}
