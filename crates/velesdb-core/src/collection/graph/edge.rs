//! Graph edge types and storage for knowledge graph relationships.
//!
//! This module provides:
//! - `GraphEdge`: A typed relationship between nodes with properties
//! - `EdgeStore`: Bidirectional index for efficient edge traversal

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A directed edge (relationship) in the knowledge graph.
///
/// Edges connect nodes and can have a label (type) and properties.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::collection::graph::GraphEdge;
/// use serde_json::json;
/// use std::collections::HashMap;
///
/// let mut props = HashMap::new();
/// props.insert("since".to_string(), json!("2020-01-01"));
///
/// let edge = GraphEdge::new(1, 100, 200, "KNOWS")
///     .with_properties(props);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    id: u64,
    source: u64,
    target: u64,
    label: String,
    properties: HashMap<String, Value>,
}

impl GraphEdge {
    /// Creates a new edge with the given ID, endpoints, and label.
    #[must_use]
    pub fn new(id: u64, source: u64, target: u64, label: &str) -> Self {
        Self {
            id,
            source,
            target,
            label: label.to_string(),
            properties: HashMap::new(),
        }
    }

    /// Adds properties to this edge (builder pattern).
    #[must_use]
    pub fn with_properties(mut self, properties: HashMap<String, Value>) -> Self {
        self.properties = properties;
        self
    }

    /// Returns the edge ID.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the source node ID.
    #[must_use]
    pub fn source(&self) -> u64 {
        self.source
    }

    /// Returns the target node ID.
    #[must_use]
    pub fn target(&self) -> u64 {
        self.target
    }

    /// Returns the edge label (relationship type).
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns all properties of this edge.
    #[must_use]
    pub fn properties(&self) -> &HashMap<String, Value> {
        &self.properties
    }

    /// Returns a specific property value, if it exists.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties.get(name)
    }
}

/// Storage for graph edges with bidirectional indexing.
///
/// Provides O(1) access to edges by ID and O(degree) access to
/// outgoing/incoming edges for any node.
#[derive(Debug, Default)]
pub struct EdgeStore {
    /// All edges indexed by ID
    edges: HashMap<u64, GraphEdge>,
    /// Outgoing edges: source_id -> Vec<edge_id>
    outgoing: HashMap<u64, Vec<u64>>,
    /// Incoming edges: target_id -> Vec<edge_id>
    incoming: HashMap<u64, Vec<u64>>,
}

impl EdgeStore {
    /// Creates a new empty edge store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an edge to the store.
    ///
    /// Creates bidirectional index entries for efficient traversal.
    pub fn add_edge(&mut self, edge: GraphEdge) {
        let id = edge.id();
        let source = edge.source();
        let target = edge.target();

        // Add to outgoing index
        self.outgoing.entry(source).or_default().push(id);

        // Add to incoming index
        self.incoming.entry(target).or_default().push(id);

        // Store the edge
        self.edges.insert(id, edge);
    }

    /// Adds an edge with only the outgoing index (for cross-shard storage).
    ///
    /// Used by `ConcurrentEdgeStore` when source and target are in different shards.
    /// The edge is stored and indexed by source node only.
    pub fn add_edge_outgoing_only(&mut self, edge: GraphEdge) {
        let id = edge.id();
        let source = edge.source();

        // Add to outgoing index only
        self.outgoing.entry(source).or_default().push(id);

        // Store the edge
        self.edges.insert(id, edge);
    }

    /// Adds an edge with only the incoming index (for cross-shard storage).
    ///
    /// Used by `ConcurrentEdgeStore` when source and target are in different shards.
    /// The edge is stored and indexed by target node only.
    pub fn add_edge_incoming_only(&mut self, edge: GraphEdge) {
        let id = edge.id();
        let target = edge.target();

        // Add to incoming index only
        self.incoming.entry(target).or_default().push(id);

        // Store the edge
        self.edges.insert(id, edge);
    }

    /// Returns the total number of edges in the store.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Returns the count of edges where this shard is the source (for accurate cross-shard counting).
    #[must_use]
    pub fn outgoing_edge_count(&self) -> usize {
        self.outgoing.values().map(Vec::len).sum()
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

    /// Gets outgoing edges filtered by label.
    #[must_use]
    pub fn get_outgoing_by_label(&self, node_id: u64, label: &str) -> Vec<&GraphEdge> {
        self.get_outgoing(node_id)
            .into_iter()
            .filter(|e| e.label() == label)
            .collect()
    }

    /// Removes an edge by ID.
    ///
    /// Cleans up both outgoing and incoming indices.
    pub fn remove_edge(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            // Remove from outgoing index
            if let Some(ids) = self.outgoing.get_mut(&edge.source()) {
                ids.retain(|&id| id != edge_id);
            }
            // Remove from incoming index
            if let Some(ids) = self.incoming.get_mut(&edge.target()) {
                ids.retain(|&id| id != edge_id);
            }
        }
    }

    /// Removes all edges connected to a node (cascade delete).
    ///
    /// Removes both outgoing and incoming edges, cleaning up all indices.
    pub fn remove_node_edges(&mut self, node_id: u64) {
        // Collect edge IDs to remove (outgoing)
        let outgoing_ids: Vec<u64> = self.outgoing.remove(&node_id).unwrap_or_default();

        // Collect edge IDs to remove (incoming)
        let incoming_ids: Vec<u64> = self.incoming.remove(&node_id).unwrap_or_default();

        // Remove outgoing edges and clean incoming indices
        for edge_id in outgoing_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                if let Some(ids) = self.incoming.get_mut(&edge.target()) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }

        // Remove incoming edges and clean outgoing indices
        for edge_id in incoming_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                if let Some(ids) = self.outgoing.get_mut(&edge.source()) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }
    }
}
