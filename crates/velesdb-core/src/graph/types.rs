//! Graph types for in-memory knowledge graph storage (no persistence dependencies).
//!
//! These types mirror `collection::graph` types but are available without the
//! `persistence` feature, enabling WASM and other non-persistence consumers
//! to work with graph data structures.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::error::{Error, Result};

/// A node in the knowledge graph.
///
/// Represents a typed entity with properties and an optional vector embedding.
///
/// # Example
///
/// ```rust
/// use velesdb_core::graph::GraphNode;
/// use serde_json::json;
/// use std::collections::HashMap;
///
/// let mut props = HashMap::new();
/// props.insert("name".to_string(), json!("Alice"));
///
/// let node = GraphNode::new(1, "Person")
///     .with_properties(props)
///     .with_vector(vec![0.1, 0.2, 0.3]);
///
/// assert_eq!(node.id(), 1);
/// assert_eq!(node.label(), "Person");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    id: u64,
    label: String,
    properties: HashMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
}

impl GraphNode {
    /// Creates a new graph node with the given ID and label.
    #[must_use]
    pub fn new(id: u64, label: &str) -> Self {
        Self {
            id,
            label: label.to_string(),
            properties: HashMap::new(),
            vector: None,
        }
    }

    /// Adds properties to this node (builder pattern).
    #[must_use]
    pub fn with_properties(mut self, properties: HashMap<String, Value>) -> Self {
        self.properties = properties;
        self
    }

    /// Adds a vector embedding to this node (builder pattern).
    #[must_use]
    pub fn with_vector(mut self, vector: Vec<f32>) -> Self {
        self.vector = Some(vector);
        self
    }

    /// Returns the node ID.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the node label (type).
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns all properties of this node.
    #[must_use]
    pub fn properties(&self) -> &HashMap<String, Value> {
        &self.properties
    }

    /// Returns a specific property value, if it exists.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties.get(name)
    }

    /// Returns the optional vector embedding.
    #[must_use]
    pub fn vector(&self) -> Option<&Vec<f32>> {
        self.vector.as_ref()
    }

    /// Sets a property value.
    pub fn set_property(&mut self, name: &str, value: Value) {
        self.properties.insert(name.to_string(), value);
    }
}

/// A directed edge (relationship) in the knowledge graph.
///
/// Edges connect nodes and can have a label (type) and properties.
///
/// # Example
///
/// ```rust
/// use velesdb_core::graph::GraphEdge;
///
/// let edge = GraphEdge::new(1, 100, 200, "KNOWS").unwrap();
/// assert_eq!(edge.source(), 100);
/// assert_eq!(edge.target(), 200);
/// assert_eq!(edge.label(), "KNOWS");
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
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidEdgeLabel` if the label is empty or whitespace-only.
    pub fn new(id: u64, source: u64, target: u64, label: &str) -> Result<Self> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(Error::InvalidEdgeLabel(
                "Edge label cannot be empty or whitespace-only".to_string(),
            ));
        }
        Ok(Self {
            id,
            source,
            target,
            label: trimmed.to_string(),
            properties: HashMap::new(),
        })
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
