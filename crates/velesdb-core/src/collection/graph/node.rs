//! Graph node and element types for knowledge graph storage.
//!
//! This module provides the core types for representing nodes in a knowledge graph:
//! - `GraphNode`: A typed entity with properties and optional vector embedding
//! - `Element`: An enum that unifies Points (vector data) and Nodes (graph entities)

use crate::Point;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A node in the knowledge graph.
///
/// Represents a typed entity with properties and an optional vector embedding.
/// Nodes are distinct from Points in that they have a label (type) and structured
/// properties, while Points are primarily vector data with metadata.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::collection::graph::GraphNode;
/// use serde_json::json;
/// use std::collections::HashMap;
///
/// let mut props = HashMap::new();
/// props.insert("name".to_string(), json!("Alice"));
/// props.insert("age".to_string(), json!(30));
///
/// let node = GraphNode::new(1, "Person")
///     .with_properties(props)
///     .with_vector(vec![0.1, 0.2, 0.3]);
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

/// A unified element that can be either a Point or a Node.
///
/// This enum allows storing both vector data (Points) and graph entities (Nodes)
/// in the same collection, enabling hybrid graph+vector storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Element {
    /// A vector point with optional metadata.
    Point(Point),
    /// A graph node with label, properties, and optional vector.
    Node(GraphNode),
}

impl Element {
    /// Returns the element ID.
    #[must_use]
    pub fn id(&self) -> u64 {
        match self {
            Self::Point(p) => p.id,
            Self::Node(n) => n.id(),
        }
    }

    /// Returns true if this is a Point.
    #[must_use]
    pub fn is_point(&self) -> bool {
        matches!(self, Self::Point(_))
    }

    /// Returns true if this is a Node.
    #[must_use]
    pub fn is_node(&self) -> bool {
        matches!(self, Self::Node(_))
    }

    /// Returns the inner Point if this is a Point.
    #[must_use]
    pub fn as_point(&self) -> Option<&Point> {
        match self {
            Self::Point(p) => Some(p),
            Self::Node(_) => None,
        }
    }

    /// Returns the inner Node if this is a Node.
    #[must_use]
    pub fn as_node(&self) -> Option<&GraphNode> {
        match self {
            Self::Point(_) => None,
            Self::Node(n) => Some(n),
        }
    }

    /// Returns true if this element has a vector embedding.
    ///
    /// Points always have vectors. Nodes may or may not have vectors.
    #[must_use]
    pub fn has_vector(&self) -> bool {
        match self {
            Self::Point(_) => true,
            Self::Node(n) => n.vector().is_some(),
        }
    }

    /// Returns the vector embedding if available.
    #[must_use]
    pub fn vector(&self) -> Option<&Vec<f32>> {
        match self {
            Self::Point(p) => Some(&p.vector),
            Self::Node(n) => n.vector(),
        }
    }
}
