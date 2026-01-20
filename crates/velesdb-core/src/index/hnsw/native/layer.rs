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
    pub(crate) fn get_neighbors(&self, node_id: NodeId) -> Vec<NodeId> {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].read().clone()
        } else {
            Vec::new()
        }
    }

    /// Sets the neighbors for a node.
    pub(crate) fn set_neighbors(&self, node_id: NodeId, neighbors: Vec<NodeId>) {
        if node_id < self.neighbors.len() {
            *self.neighbors[node_id].write() = neighbors;
        }
    }

    /// Adds a neighbor to a node's adjacency list.
    #[allow(dead_code)]
    pub(super) fn add_neighbor(&self, node_id: NodeId, neighbor: NodeId) {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].write().push(neighbor);
        }
    }
}
