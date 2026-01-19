//! Graph bindings for VelesDB WASM.
//!
//! Provides wasm-bindgen wrappers for graph operations (nodes, edges, traversal).
//! Enables knowledge graph construction in browser applications.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// A graph node for knowledge graph construction.
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

/// In-memory graph store for browser-based knowledge graphs.
///
/// Stores nodes and edges with bidirectional indexing for efficient traversal.
#[wasm_bindgen]
pub struct GraphStore {
    nodes: std::collections::HashMap<u64, GraphNode>,
    edges: std::collections::HashMap<u64, GraphEdge>,
    outgoing: std::collections::HashMap<u64, Vec<u64>>,
    incoming: std::collections::HashMap<u64, Vec<u64>>,
}

#[wasm_bindgen]
impl GraphStore {
    /// Creates a new empty graph store.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            nodes: std::collections::HashMap::new(),
            edges: std::collections::HashMap::new(),
            outgoing: std::collections::HashMap::new(),
            incoming: std::collections::HashMap::new(),
        }
    }

    /// Adds a node to the graph.
    #[wasm_bindgen]
    pub fn add_node(&mut self, node: GraphNode) {
        self.nodes.insert(node.id, node);
    }

    /// Adds an edge to the graph.
    ///
    /// Returns an error if an edge with the same ID already exists.
    #[wasm_bindgen]
    pub fn add_edge(&mut self, edge: GraphEdge) -> Result<(), JsValue> {
        if self.edges.contains_key(&edge.id) {
            return Err(JsValue::from_str(&format!(
                "Edge with ID {} already exists",
                edge.id
            )));
        }

        let source = edge.source;
        let target = edge.target;
        let id = edge.id;

        self.edges.insert(id, edge);
        self.outgoing.entry(source).or_default().push(id);
        self.incoming.entry(target).or_default().push(id);

        Ok(())
    }

    /// Gets a node by ID.
    #[wasm_bindgen]
    pub fn get_node(&self, id: u64) -> Option<GraphNode> {
        self.nodes.get(&id).cloned()
    }

    /// Gets an edge by ID.
    #[wasm_bindgen]
    pub fn get_edge(&self, id: u64) -> Option<GraphEdge> {
        self.edges.get(&id).cloned()
    }

    /// Returns the number of nodes.
    #[wasm_bindgen(getter)]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns the number of edges.
    #[wasm_bindgen(getter)]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Gets outgoing edges from a node.
    #[wasm_bindgen]
    pub fn get_outgoing(&self, node_id: u64) -> Vec<GraphEdge> {
        self.outgoing
            .get(&node_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.edges.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets incoming edges to a node.
    #[wasm_bindgen]
    pub fn get_incoming(&self, node_id: u64) -> Vec<GraphEdge> {
        self.incoming
            .get(&node_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.edges.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets outgoing edges filtered by label.
    #[wasm_bindgen]
    pub fn get_outgoing_by_label(&self, node_id: u64, label: &str) -> Vec<GraphEdge> {
        self.get_outgoing(node_id)
            .into_iter()
            .filter(|e| e.label == label)
            .collect()
    }

    /// Gets neighbors reachable from a node (1-hop).
    #[wasm_bindgen]
    pub fn get_neighbors(&self, node_id: u64) -> Vec<u64> {
        self.get_outgoing(node_id)
            .into_iter()
            .map(|e| e.target)
            .collect()
    }

    /// Performs BFS traversal from a source node.
    ///
    /// # Arguments
    ///
    /// * `source_id` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Array of reachable node IDs with their depths.
    #[wasm_bindgen]
    pub fn bfs_traverse(
        &self,
        source_id: u64,
        max_depth: usize,
        limit: usize,
    ) -> Result<JsValue, JsValue> {
        use std::collections::{HashSet, VecDeque};

        let mut results: Vec<(u64, usize)> = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut queue: VecDeque<(u64, usize)> = VecDeque::new();

        queue.push_back((source_id, 0));
        visited.insert(source_id);

        while let Some((node_id, depth)) = queue.pop_front() {
            if results.len() >= limit {
                break;
            }

            if depth > 0 {
                results.push((node_id, depth));
            }

            if depth < max_depth {
                for edge in self.get_outgoing(node_id) {
                    if !visited.contains(&edge.target) {
                        visited.insert(edge.target);
                        queue.push_back((edge.target, depth + 1));
                    }
                }
            }
        }

        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Removes a node and all connected edges.
    #[wasm_bindgen]
    pub fn remove_node(&mut self, node_id: u64) {
        self.nodes.remove(&node_id);

        let outgoing_ids: Vec<u64> = self.outgoing.remove(&node_id).unwrap_or_default();
        for edge_id in outgoing_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                if let Some(ids) = self.incoming.get_mut(&edge.target) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }

        let incoming_ids: Vec<u64> = self.incoming.remove(&node_id).unwrap_or_default();
        for edge_id in incoming_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                if let Some(ids) = self.outgoing.get_mut(&edge.source) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }
    }

    /// Removes an edge by ID.
    #[wasm_bindgen]
    pub fn remove_edge(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            if let Some(ids) = self.outgoing.get_mut(&edge.source) {
                ids.retain(|&id| id != edge_id);
            }
            if let Some(ids) = self.incoming.get_mut(&edge.target) {
                ids.retain(|&id| id != edge_id);
            }
        }
    }

    /// Clears all nodes and edges.
    #[wasm_bindgen]
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.outgoing.clear();
        self.incoming.clear();
    }
}

impl Default for GraphStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn test_graph_node_creation() {
        let node = GraphNode::new(1, "Person");
        assert_eq!(node.id(), 1);
        assert_eq!(node.label(), "Person");
        assert!(!node.has_vector());
    }

    #[wasm_bindgen_test]
    fn test_graph_node_properties() {
        let mut node = GraphNode::new(1, "Person");
        node.set_string_property("name", "John");
        node.set_number_property("age", 30.0);
        node.set_bool_property("active", true);

        assert!(node.properties.contains_key("name"));
        assert!(node.properties.contains_key("age"));
        assert!(node.properties.contains_key("active"));
    }

    #[wasm_bindgen_test]
    fn test_graph_node_vector() {
        let mut node = GraphNode::new(1, "Document");
        assert!(!node.has_vector());

        node.set_vector(vec![0.1, 0.2, 0.3]);
        assert!(node.has_vector());
    }

    #[wasm_bindgen_test]
    fn test_graph_edge_creation() {
        let edge = GraphEdge::new(100, 1, 2, "KNOWS").unwrap();
        assert_eq!(edge.id(), 100);
        assert_eq!(edge.source(), 1);
        assert_eq!(edge.target(), 2);
        assert_eq!(edge.label(), "KNOWS");
    }

    #[wasm_bindgen_test]
    fn test_graph_edge_empty_label_error() {
        let result = GraphEdge::new(100, 1, 2, "  ");
        assert!(result.is_err());
    }

    #[wasm_bindgen_test]
    fn test_graph_store_add_nodes() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));

        assert_eq!(store.node_count(), 2);
    }

    #[wasm_bindgen_test]
    fn test_graph_store_add_edges() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();

        assert_eq!(store.edge_count(), 1);
    }

    #[wasm_bindgen_test]
    fn test_graph_store_get_outgoing() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store.add_node(GraphNode::new(3, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(101, 1, 3, "KNOWS").unwrap())
            .unwrap();

        let outgoing = store.get_outgoing(1);
        assert_eq!(outgoing.len(), 2);
    }

    #[wasm_bindgen_test]
    fn test_graph_store_get_neighbors() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store.add_node(GraphNode::new(3, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(101, 1, 3, "KNOWS").unwrap())
            .unwrap();

        let neighbors = store.get_neighbors(1);
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(&2));
        assert!(neighbors.contains(&3));
    }

    #[wasm_bindgen_test]
    fn test_graph_store_remove_node() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();

        store.remove_node(1);

        assert_eq!(store.node_count(), 1);
        assert_eq!(store.edge_count(), 0);
    }

    #[wasm_bindgen_test]
    fn test_graph_store_duplicate_edge_error() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();

        let result = store.add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap());
        assert!(result.is_err());
    }

    #[wasm_bindgen_test]
    fn test_graph_store_get_node() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));

        let node = store.get_node(1);
        assert!(node.is_some());
        assert_eq!(node.unwrap().id(), 1);

        let missing = store.get_node(999);
        assert!(missing.is_none());
    }

    #[wasm_bindgen_test]
    fn test_graph_store_get_edge() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();

        let edge = store.get_edge(100);
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().label(), "KNOWS");

        let missing = store.get_edge(999);
        assert!(missing.is_none());
    }

    #[wasm_bindgen_test]
    fn test_graph_store_get_incoming() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store.add_node(GraphNode::new(3, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 3, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
            .unwrap();

        let incoming = store.get_incoming(3);
        assert_eq!(incoming.len(), 2);
    }

    #[wasm_bindgen_test]
    fn test_graph_store_get_outgoing_by_label() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store.add_node(GraphNode::new(3, "Company"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();
        store
            .add_edge(GraphEdge::new(101, 1, 3, "WORKS_AT").unwrap())
            .unwrap();

        let knows = store.get_outgoing_by_label(1, "KNOWS");
        assert_eq!(knows.len(), 1);
        assert_eq!(knows[0].target(), 2);

        let works = store.get_outgoing_by_label(1, "WORKS_AT");
        assert_eq!(works.len(), 1);
        assert_eq!(works[0].target(), 3);
    }

    #[wasm_bindgen_test]
    fn test_graph_store_remove_edge() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();

        assert_eq!(store.edge_count(), 1);
        store.remove_edge(100);
        assert_eq!(store.edge_count(), 0);
        assert!(store.get_outgoing(1).is_empty());
    }

    #[wasm_bindgen_test]
    fn test_graph_store_clear() {
        let mut store = GraphStore::new();
        store.add_node(GraphNode::new(1, "Person"));
        store.add_node(GraphNode::new(2, "Person"));
        store
            .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
            .unwrap();

        assert_eq!(store.node_count(), 2);
        assert_eq!(store.edge_count(), 1);

        store.clear();

        assert_eq!(store.node_count(), 0);
        assert_eq!(store.edge_count(), 0);
    }
}
