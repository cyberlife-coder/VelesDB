//! Graph bindings for `VelesDB` WASM.
//!
//! Provides wasm-bindgen wrappers for graph operations (nodes, edges, traversal).
//! Enables knowledge graph construction in browser applications.

use serde::{Deserialize, Serialize};
use velesdb_core::graph as core_graph;
use wasm_bindgen::prelude::*;

/// A graph node for knowledge graph construction.
#[allow(clippy::unsafe_derive_deserialize)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[wasm_bindgen]
pub struct GraphNode {
    id: u64,
    label: String,
    properties: std::collections::HashMap<String, serde_json::Value>,
    vector: Option<Vec<f32>>,
}

#[wasm_bindgen]
impl GraphNode {
    /// Creates a new graph node.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the node
    /// * `label` - Node type/label (e.g., "Person", "Document")
    #[wasm_bindgen(constructor)]
    pub fn new(id: u64, label: &str) -> Self {
        Self {
            id,
            label: label.to_string(),
            properties: std::collections::HashMap::new(),
            vector: None,
        }
    }

    /// Returns the node ID.
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the node label.
    #[wasm_bindgen(getter)]
    pub fn label(&self) -> String {
        self.label.clone()
    }

    /// Sets a string property on the node.
    #[wasm_bindgen]
    pub fn set_string_property(&mut self, key: &str, value: &str) {
        self.properties.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }

    /// Sets a numeric property on the node.
    #[wasm_bindgen]
    pub fn set_number_property(&mut self, key: &str, value: f64) {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.properties
                .insert(key.to_string(), serde_json::Value::Number(n));
        }
    }

    /// Sets a boolean property on the node.
    #[wasm_bindgen]
    pub fn set_bool_property(&mut self, key: &str, value: bool) {
        self.properties
            .insert(key.to_string(), serde_json::Value::Bool(value));
    }

    /// Sets a vector embedding on the node.
    #[wasm_bindgen]
    pub fn set_vector(&mut self, vector: Vec<f32>) {
        self.vector = Some(vector);
    }

    /// Returns true if this node has a vector embedding.
    #[wasm_bindgen]
    pub fn has_vector(&self) -> bool {
        self.vector.is_some()
    }

    /// Converts to JSON for JavaScript interop.
    #[wasm_bindgen]
    pub fn to_json(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(self).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

/// A graph edge representing a relationship between nodes.
#[allow(clippy::unsafe_derive_deserialize)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[wasm_bindgen]
pub struct GraphEdge {
    id: u64,
    source: u64,
    target: u64,
    label: String,
    properties: std::collections::HashMap<String, serde_json::Value>,
}

#[wasm_bindgen]
impl GraphEdge {
    /// Creates a new graph edge.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the edge
    /// * `source` - Source node ID
    /// * `target` - Target node ID
    /// * `label` - Relationship type (e.g., "KNOWS", "WROTE")
    #[wasm_bindgen(constructor)]
    pub fn new(id: u64, source: u64, target: u64, label: &str) -> Result<GraphEdge, JsValue> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(JsValue::from_str("Edge label cannot be empty"));
        }

        Ok(Self {
            id,
            source,
            target,
            label: trimmed.to_string(),
            properties: std::collections::HashMap::new(),
        })
    }

    /// Returns the edge ID.
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the source node ID.
    #[wasm_bindgen(getter)]
    pub fn source(&self) -> u64 {
        self.source
    }

    /// Returns the target node ID.
    #[wasm_bindgen(getter)]
    pub fn target(&self) -> u64 {
        self.target
    }

    /// Returns the edge label (relationship type).
    #[wasm_bindgen(getter)]
    pub fn label(&self) -> String {
        self.label.clone()
    }

    /// Sets a string property on the edge.
    #[wasm_bindgen]
    pub fn set_string_property(&mut self, key: &str, value: &str) {
        self.properties.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }

    /// Sets a numeric property on the edge.
    #[wasm_bindgen]
    pub fn set_number_property(&mut self, key: &str, value: f64) {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.properties
                .insert(key.to_string(), serde_json::Value::Number(n));
        }
    }

    /// Converts to JSON for JavaScript interop.
    #[wasm_bindgen]
    pub fn to_json(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(self).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

// ── Conversion helpers: WASM ↔ Core ────────────────────────────────

fn wasm_node_to_core(node: &GraphNode) -> core_graph::GraphNode {
    let mut core =
        core_graph::GraphNode::new(node.id, &node.label).with_properties(node.properties.clone());
    if let Some(ref v) = node.vector {
        core = core.with_vector(v.clone());
    }
    core
}

fn core_node_to_wasm(core: &core_graph::GraphNode) -> GraphNode {
    GraphNode {
        id: core.id(),
        label: core.label().to_string(),
        properties: core.properties().clone(),
        vector: core.vector().cloned(),
    }
}

fn wasm_edge_to_core(edge: &GraphEdge) -> Option<core_graph::GraphEdge> {
    core_graph::GraphEdge::new(edge.id, edge.source, edge.target, &edge.label)
        .ok()
        .map(|e| e.with_properties(edge.properties.clone()))
}

fn core_edge_to_wasm(core: &core_graph::GraphEdge) -> GraphEdge {
    GraphEdge {
        id: core.id(),
        source: core.source(),
        target: core.target(),
        label: core.label().to_string(),
        properties: core.properties().clone(),
    }
}

/// In-memory graph store for browser-based knowledge graphs.
///
/// Delegates to core's `InMemoryEdgeStore` for storage and traversal.
#[wasm_bindgen]
pub struct GraphStore {
    store: core_graph::InMemoryEdgeStore,
}

#[wasm_bindgen]
impl GraphStore {
    /// Creates a new empty graph store.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            store: core_graph::InMemoryEdgeStore::new(),
        }
    }

    /// Adds a node to the graph.
    #[wasm_bindgen]
    pub fn add_node(&mut self, node: GraphNode) {
        let core_node = wasm_node_to_core(&node);
        // Silently overwrite if exists (matches previous WASM behavior)
        if self.store.has_node(node.id) {
            self.store.remove_node(node.id);
        }
        let _ = self.store.add_node(core_node);
    }

    /// Adds an edge to the graph.
    ///
    /// Returns an error if an edge with the same ID already exists.
    #[wasm_bindgen]
    pub fn add_edge(&mut self, edge: GraphEdge) -> Result<(), JsValue> {
        let core_edge =
            wasm_edge_to_core(&edge).ok_or_else(|| JsValue::from_str("Invalid edge label"))?;
        self.store
            .add_edge(core_edge)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Gets a node by ID.
    #[wasm_bindgen]
    pub fn get_node(&self, id: u64) -> Option<GraphNode> {
        self.store.get_node(id).map(core_node_to_wasm)
    }

    /// Gets an edge by ID.
    #[wasm_bindgen]
    pub fn get_edge(&self, id: u64) -> Option<GraphEdge> {
        self.store.get_edge(id).map(core_edge_to_wasm)
    }

    /// Returns the number of nodes.
    #[wasm_bindgen(getter)]
    pub fn node_count(&self) -> usize {
        self.store.node_count()
    }

    /// Returns the number of edges.
    #[wasm_bindgen(getter)]
    pub fn edge_count(&self) -> usize {
        self.store.edge_count()
    }

    /// Gets outgoing edges from a node.
    #[wasm_bindgen]
    pub fn get_outgoing(&self, node_id: u64) -> Vec<GraphEdge> {
        self.store
            .get_outgoing(node_id)
            .iter()
            .map(|e| core_edge_to_wasm(e))
            .collect()
    }

    /// Gets incoming edges to a node.
    #[wasm_bindgen]
    pub fn get_incoming(&self, node_id: u64) -> Vec<GraphEdge> {
        self.store
            .get_incoming(node_id)
            .iter()
            .map(|e| core_edge_to_wasm(e))
            .collect()
    }

    /// Gets outgoing edges filtered by label.
    #[wasm_bindgen]
    pub fn get_outgoing_by_label(&self, node_id: u64, label: &str) -> Vec<GraphEdge> {
        self.store
            .get_outgoing_by_label(node_id, label)
            .iter()
            .map(|e| core_edge_to_wasm(e))
            .collect()
    }

    /// Gets neighbors reachable from a node (1-hop).
    #[wasm_bindgen]
    pub fn get_neighbors(&self, node_id: u64) -> Vec<u64> {
        self.store
            .get_outgoing(node_id)
            .iter()
            .map(|e| e.target())
            .collect()
    }

    /// Performs BFS traversal from a source node.
    ///
    /// Delegates to core's BFS via `GraphTraversal` trait.
    #[wasm_bindgen]
    pub fn bfs_traverse(
        &self,
        source_id: u64,
        max_depth: usize,
        limit: usize,
    ) -> Result<JsValue, JsValue> {
        let config = core_graph::TraversalConfig::new(max_depth, limit);
        let steps = core_graph::traversal::bfs(&self.store, source_id, &config);
        let results: Vec<(u64, usize)> = steps.iter().map(|s| (s.node_id, s.depth)).collect();
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Removes a node and all connected edges.
    #[wasm_bindgen]
    pub fn remove_node(&mut self, node_id: u64) {
        self.store.remove_node(node_id);
    }

    /// Removes an edge by ID.
    #[wasm_bindgen]
    pub fn remove_edge(&mut self, edge_id: u64) {
        self.store.remove_edge(edge_id);
    }

    /// Clears all nodes and edges.
    #[wasm_bindgen]
    pub fn clear(&mut self) {
        self.store.clear();
    }

    /// Performs DFS traversal from a source node.
    ///
    /// Delegates to core's DFS via `GraphTraversal` trait.
    #[wasm_bindgen]
    pub fn dfs_traverse(
        &self,
        source_id: u64,
        max_depth: usize,
        limit: usize,
    ) -> Result<JsValue, JsValue> {
        let config = core_graph::TraversalConfig::new(max_depth, limit);
        let steps = core_graph::traversal::dfs(&self.store, source_id, &config);
        let results: Vec<(u64, usize)> = steps.iter().map(|s| (s.node_id, s.depth)).collect();
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Gets all nodes with a specific label.
    #[wasm_bindgen]
    pub fn get_nodes_by_label(&self, label: &str) -> Vec<GraphNode> {
        self.store
            .get_nodes_by_label(label)
            .iter()
            .map(|n| core_node_to_wasm(n))
            .collect()
    }

    /// Gets all edges with a specific label.
    #[wasm_bindgen]
    pub fn get_edges_by_label(&self, label: &str) -> Vec<GraphEdge> {
        self.store
            .get_edges_by_label(label)
            .iter()
            .map(|e| core_edge_to_wasm(e))
            .collect()
    }

    /// Gets all node IDs in the graph.
    #[wasm_bindgen]
    pub fn get_all_node_ids(&self) -> Vec<u64> {
        self.store.all_node_ids()
    }

    /// Gets all edge IDs in the graph.
    #[wasm_bindgen]
    pub fn get_all_edge_ids(&self) -> Vec<u64> {
        self.store.all_edge_ids()
    }

    /// Checks if a node exists.
    #[wasm_bindgen]
    pub fn has_node(&self, id: u64) -> bool {
        self.store.has_node(id)
    }

    /// Checks if an edge exists.
    #[wasm_bindgen]
    pub fn has_edge(&self, id: u64) -> bool {
        self.store.has_edge(id)
    }

    /// Gets the degree (number of outgoing edges) of a node.
    #[wasm_bindgen]
    pub fn out_degree(&self, node_id: u64) -> usize {
        self.store.out_degree(node_id)
    }

    /// Gets the in-degree (number of incoming edges) of a node.
    #[wasm_bindgen]
    pub fn in_degree(&self, node_id: u64) -> usize {
        self.store.in_degree(node_id)
    }
}

/// Internal methods for `GraphStore` (not exposed to WASM).
impl GraphStore {
    /// Returns all nodes in the graph (for persistence - internal use).
    pub(crate) fn get_all_nodes_internal(&self) -> Vec<GraphNode> {
        self.store
            .all_nodes()
            .iter()
            .map(|n| core_node_to_wasm(n))
            .collect()
    }

    /// Returns all edges in the graph (for persistence - internal use).
    pub(crate) fn get_all_edges_internal(&self) -> Vec<GraphEdge> {
        self.store
            .all_edges()
            .iter()
            .map(|e| core_edge_to_wasm(e))
            .collect()
    }
}

impl Default for GraphStore {
    fn default() -> Self {
        Self::new()
    }
}

// NOTE: Tests moved to graph_tests.rs (EPIC-061/US-006 refactoring)
#[cfg(test)]
#[path = "graph_tests.rs"]
mod graph_tests;
