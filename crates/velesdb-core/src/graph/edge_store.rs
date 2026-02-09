//! In-memory edge store for graph data (no persistence dependencies).
//!
//! Provides bidirectional indexing for efficient graph traversal.
//! This is the non-persistence equivalent of `collection::graph::EdgeStore`,
//! usable by WASM and other consumers that don't need filesystem storage.

use std::collections::HashMap;

use crate::error::{Error, Result};

use super::types::{GraphEdge, GraphNode};

/// In-memory storage for graph nodes and edges with bidirectional indexing.
///
/// Provides O(1) access to nodes/edges by ID and O(degree) access to
/// outgoing/incoming edges for any node.
#[derive(Debug, Default)]
pub struct InMemoryEdgeStore {
    /// All nodes indexed by ID.
    nodes: HashMap<u64, GraphNode>,
    /// All edges indexed by ID.
    edges: HashMap<u64, GraphEdge>,
    /// Outgoing edges: source_id -> Vec<edge_id>.
    outgoing: HashMap<u64, Vec<u64>>,
    /// Incoming edges: target_id -> Vec<edge_id>.
    incoming: HashMap<u64, Vec<u64>>,
    /// Secondary index: label -> Vec<edge_id> for fast label queries.
    by_label: HashMap<String, Vec<u64>>,
    /// Composite index: (source_id, label) -> Vec<edge_id>.
    outgoing_by_label: HashMap<(u64, String), Vec<u64>>,
}

impl InMemoryEdgeStore {
    /// Creates a new empty edge store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an edge store with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(expected_edges: usize, expected_nodes: usize) -> Self {
        let expected_labels = 10usize;
        let outgoing_by_label_cap = expected_nodes
            .saturating_mul(expected_labels)
            .saturating_div(10);
        Self {
            nodes: HashMap::with_capacity(expected_nodes),
            edges: HashMap::with_capacity(expected_edges),
            outgoing: HashMap::with_capacity(expected_nodes),
            incoming: HashMap::with_capacity(expected_nodes),
            by_label: HashMap::with_capacity(expected_labels),
            outgoing_by_label: HashMap::with_capacity(outgoing_by_label_cap),
        }
    }

    // ── Node CRUD ──────────────────────────────────────────────────────

    /// Adds a node to the store.
    ///
    /// # Errors
    ///
    /// Returns `Error::NodeExists` if a node with the same ID already exists.
    pub fn add_node(&mut self, node: GraphNode) -> Result<()> {
        let id = node.id();
        if self.nodes.contains_key(&id) {
            return Err(Error::NodeExists(id));
        }
        self.nodes.insert(id, node);
        Ok(())
    }

    /// Gets a node by ID.
    #[must_use]
    pub fn get_node(&self, id: u64) -> Option<&GraphNode> {
        self.nodes.get(&id)
    }

    /// Gets a mutable reference to a node by ID.
    #[must_use]
    pub fn get_node_mut(&mut self, id: u64) -> Option<&mut GraphNode> {
        self.nodes.get_mut(&id)
    }

    /// Removes a node and all its connected edges (cascade delete).
    pub fn remove_node(&mut self, node_id: u64) -> Option<GraphNode> {
        let node = self.nodes.remove(&node_id)?;
        self.remove_node_edges(node_id);
        Some(node)
    }

    /// Returns true if a node with the given ID exists.
    #[must_use]
    pub fn has_node(&self, node_id: u64) -> bool {
        self.nodes.contains_key(&node_id)
    }

    /// Returns the total number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns all node IDs.
    #[must_use]
    pub fn all_node_ids(&self) -> Vec<u64> {
        self.nodes.keys().copied().collect()
    }

    /// Returns all nodes.
    #[must_use]
    pub fn all_nodes(&self) -> Vec<&GraphNode> {
        self.nodes.values().collect()
    }

    /// Returns nodes filtered by label.
    #[must_use]
    pub fn get_nodes_by_label(&self, label: &str) -> Vec<&GraphNode> {
        self.nodes.values().filter(|n| n.label() == label).collect()
    }

    // ── Edge CRUD ──────────────────────────────────────────────────────

    /// Adds an edge to the store.
    ///
    /// Creates bidirectional index entries for efficient traversal.
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    pub fn add_edge(&mut self, edge: GraphEdge) -> Result<()> {
        let id = edge.id();
        let source = edge.source();
        let target = edge.target();
        let label = edge.label().to_string();

        if self.edges.contains_key(&id) {
            return Err(Error::EdgeExists(id));
        }

        self.outgoing.entry(source).or_default().push(id);
        self.incoming.entry(target).or_default().push(id);
        self.by_label.entry(label.clone()).or_default().push(id);
        self.outgoing_by_label
            .entry((source, label))
            .or_default()
            .push(id);

        self.edges.insert(id, edge);
        Ok(())
    }

    /// Gets an edge by its ID.
    #[must_use]
    pub fn get_edge(&self, id: u64) -> Option<&GraphEdge> {
        self.edges.get(&id)
    }

    /// Gets all outgoing edges from a node.
    #[must_use]
    pub fn get_outgoing(&self, node_id: u64) -> Vec<&GraphEdge> {
        self.outgoing
            .get(&node_id)
            .map(|ids| ids.iter().filter_map(|id| self.edges.get(id)).collect())
            .unwrap_or_default()
    }

    /// Gets all incoming edges to a node.
    #[must_use]
    pub fn get_incoming(&self, node_id: u64) -> Vec<&GraphEdge> {
        self.incoming
            .get(&node_id)
            .map(|ids| ids.iter().filter_map(|id| self.edges.get(id)).collect())
            .unwrap_or_default()
    }

    /// Gets outgoing edges filtered by label using composite index.
    #[must_use]
    pub fn get_outgoing_by_label(&self, node_id: u64, label: &str) -> Vec<&GraphEdge> {
        self.outgoing_by_label
            .get(&(node_id, label.to_string()))
            .map(|ids| ids.iter().filter_map(|id| self.edges.get(id)).collect())
            .unwrap_or_default()
    }

    /// Gets all edges with a specific label.
    #[must_use]
    pub fn get_edges_by_label(&self, label: &str) -> Vec<&GraphEdge> {
        self.by_label
            .get(label)
            .map(|ids| ids.iter().filter_map(|id| self.edges.get(id)).collect())
            .unwrap_or_default()
    }

    /// Checks if an edge with the given ID exists.
    #[must_use]
    pub fn has_edge(&self, edge_id: u64) -> bool {
        self.edges.contains_key(&edge_id)
    }

    /// Returns the total number of edges.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Returns all edge IDs.
    #[must_use]
    pub fn all_edge_ids(&self) -> Vec<u64> {
        self.edges.keys().copied().collect()
    }

    /// Returns all edges.
    #[must_use]
    pub fn all_edges(&self) -> Vec<&GraphEdge> {
        self.edges.values().collect()
    }

    /// Returns the out-degree of a node.
    #[must_use]
    pub fn out_degree(&self, node_id: u64) -> usize {
        self.outgoing.get(&node_id).map_or(0, Vec::len)
    }

    /// Returns the in-degree of a node.
    #[must_use]
    pub fn in_degree(&self, node_id: u64) -> usize {
        self.incoming.get(&node_id).map_or(0, Vec::len)
    }

    /// Removes an edge by ID, cleaning up all indices.
    pub fn remove_edge(&mut self, edge_id: u64) -> Option<GraphEdge> {
        let edge = self.edges.remove(&edge_id)?;
        let source = edge.source();
        let target = edge.target();
        let label = edge.label().to_string();

        if let Some(ids) = self.outgoing.get_mut(&source) {
            ids.retain(|&id| id != edge_id);
        }
        if let Some(ids) = self.incoming.get_mut(&target) {
            ids.retain(|&id| id != edge_id);
        }
        if let Some(ids) = self.by_label.get_mut(&label) {
            ids.retain(|&id| id != edge_id);
        }
        if let Some(ids) = self.outgoing_by_label.get_mut(&(source, label)) {
            ids.retain(|&id| id != edge_id);
        }

        Some(edge)
    }

    /// Clears all nodes and edges from the store.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.outgoing.clear();
        self.incoming.clear();
        self.by_label.clear();
        self.outgoing_by_label.clear();
    }

    /// Removes all edges connected to a node (cascade delete).
    fn remove_node_edges(&mut self, node_id: u64) {
        let outgoing_ids: Vec<u64> = self.outgoing.remove(&node_id).unwrap_or_default();
        let incoming_ids: Vec<u64> = self.incoming.remove(&node_id).unwrap_or_default();

        for edge_id in outgoing_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                let label = edge.label().to_string();
                if let Some(ids) = self.incoming.get_mut(&edge.target()) {
                    ids.retain(|&id| id != edge_id);
                }
                if let Some(ids) = self.by_label.get_mut(&label) {
                    ids.retain(|&id| id != edge_id);
                }
                if let Some(ids) = self.outgoing_by_label.get_mut(&(node_id, label)) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }

        for edge_id in incoming_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                let source = edge.source();
                let label = edge.label().to_string();
                if let Some(ids) = self.outgoing.get_mut(&source) {
                    ids.retain(|&id| id != edge_id);
                }
                if let Some(ids) = self.by_label.get_mut(&label) {
                    ids.retain(|&id| id != edge_id);
                }
                if let Some(ids) = self.outgoing_by_label.get_mut(&(source, label)) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }
    }
}
