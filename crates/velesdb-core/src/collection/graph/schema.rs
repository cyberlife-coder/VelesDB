//! Graph schema definitions for heterogeneous knowledge graphs.
//!
//! This module provides schema definitions for graph collections,
//! supporting both strict schemas (with predefined node/edge types)
//! and schemaless mode (accepting arbitrary types).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{Error, Result};

/// Value types supported for node and edge properties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    /// String value.
    String,
    /// Integer value (i64).
    Integer,
    /// Floating-point value (f64).
    Float,
    /// Boolean value.
    Boolean,
    /// Vector embedding (for hybrid graph+vector queries).
    Vector,
}

/// Definition of a node type in the graph schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeType {
    name: String,
    properties: HashMap<String, ValueType>,
}

impl NodeType {
    /// Creates a new node type with the given name.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            properties: HashMap::new(),
        }
    }

    /// Adds properties to this node type (builder pattern).
    #[must_use]
    pub fn with_properties(mut self, properties: HashMap<String, ValueType>) -> Self {
        self.properties = properties;
        self
    }

    /// Returns the name of this node type.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns all properties of this node type.
    #[must_use]
    pub fn properties(&self) -> &HashMap<String, ValueType> {
        &self.properties
    }

    /// Returns the type of a specific property, if it exists.
    #[must_use]
    pub fn property_type(&self, name: &str) -> Option<&ValueType> {
        self.properties.get(name)
    }
}

/// Definition of an edge type in the graph schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeType {
    name: String,
    from_type: String,
    to_type: String,
    properties: HashMap<String, ValueType>,
}

impl EdgeType {
    /// Creates a new edge type with the given name and endpoint types.
    #[must_use]
    pub fn new(name: &str, from_type: &str, to_type: &str) -> Self {
        Self {
            name: name.to_string(),
            from_type: from_type.to_string(),
            to_type: to_type.to_string(),
            properties: HashMap::new(),
        }
    }

    /// Adds properties to this edge type (builder pattern).
    #[must_use]
    pub fn with_properties(mut self, properties: HashMap<String, ValueType>) -> Self {
        self.properties = properties;
        self
    }

    /// Returns the name of this edge type.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the source node type.
    #[must_use]
    pub fn from_type(&self) -> &str {
        &self.from_type
    }

    /// Returns the target node type.
    #[must_use]
    pub fn to_type(&self) -> &str {
        &self.to_type
    }

    /// Returns all properties of this edge type.
    #[must_use]
    pub fn properties(&self) -> &HashMap<String, ValueType> {
        &self.properties
    }
}

/// Schema for a graph collection.
///
/// Supports two modes:
/// - **Strict mode**: Only predefined node/edge types are allowed.
/// - **Schemaless mode**: Any node/edge type is accepted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphSchema {
    schemaless: bool,
    node_types: Vec<NodeType>,
    edge_types: Vec<EdgeType>,
}

impl Default for GraphSchema {
    fn default() -> Self {
        Self::schemaless()
    }
}

impl GraphSchema {
    /// Creates a new strict (non-schemaless) graph schema.
    ///
    /// Use `with_node_type` and `with_edge_type` to add allowed types.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemaless: false,
            node_types: Vec::new(),
            edge_types: Vec::new(),
        }
    }

    /// Creates a schemaless graph schema that accepts any types.
    #[must_use]
    pub fn schemaless() -> Self {
        Self {
            schemaless: true,
            node_types: Vec::new(),
            edge_types: Vec::new(),
        }
    }

    /// Adds a node type to the schema (builder pattern).
    #[must_use]
    pub fn with_node_type(mut self, node_type: NodeType) -> Self {
        self.node_types.push(node_type);
        self
    }

    /// Adds an edge type to the schema (builder pattern).
    #[must_use]
    pub fn with_edge_type(mut self, edge_type: EdgeType) -> Self {
        self.edge_types.push(edge_type);
        self
    }

    /// Returns whether this schema is schemaless.
    #[must_use]
    pub fn is_schemaless(&self) -> bool {
        self.schemaless
    }

    /// Returns all node types in this schema.
    #[must_use]
    pub fn node_types(&self) -> &[NodeType] {
        &self.node_types
    }

    /// Returns all edge types in this schema.
    #[must_use]
    pub fn edge_types(&self) -> &[EdgeType] {
        &self.edge_types
    }

    /// Checks if a node type exists in this schema.
    #[must_use]
    pub fn has_node_type(&self, name: &str) -> bool {
        self.node_types.iter().any(|nt| nt.name == name)
    }

    /// Checks if an edge type exists in this schema.
    #[must_use]
    pub fn has_edge_type(&self, name: &str) -> bool {
        self.edge_types.iter().any(|et| et.name == name)
    }

    /// Validates a node type against this schema.
    ///
    /// Returns `Ok(())` if the type is valid, or an error with details.
    pub fn validate_node_type(&self, type_name: &str) -> Result<()> {
        if self.schemaless {
            return Ok(());
        }

        if self.has_node_type(type_name) {
            return Ok(());
        }

        let allowed: Vec<&str> = self.node_types.iter().map(|nt| nt.name.as_str()).collect();
        Err(Error::SchemaValidation(format!(
            "Node type '{}' not allowed. Valid types: {:?}",
            type_name, allowed
        )))
    }

    /// Validates an edge type against this schema.
    ///
    /// Checks the edge type name and that source/target node types are valid.
    pub fn validate_edge_type(
        &self,
        edge_type: &str,
        from_type: &str,
        to_type: &str,
    ) -> Result<()> {
        if self.schemaless {
            return Ok(());
        }

        // Find the edge type definition
        let edge_def = self.edge_types.iter().find(|et| et.name == edge_type);

        match edge_def {
            Some(def) => {
                // Validate source node type
                if def.from_type != from_type {
                    return Err(Error::SchemaValidation(format!(
                        "Edge '{}' expects source type '{}', got '{}'",
                        edge_type, def.from_type, from_type
                    )));
                }
                // Validate target node type
                if def.to_type != to_type {
                    return Err(Error::SchemaValidation(format!(
                        "Edge '{}' expects target type '{}', got '{}'",
                        edge_type, def.to_type, to_type
                    )));
                }
                Ok(())
            }
            None => {
                let allowed: Vec<&str> =
                    self.edge_types.iter().map(|et| et.name.as_str()).collect();
                Err(Error::SchemaValidation(format!(
                    "Edge type '{}' not allowed. Valid types: {:?}",
                    edge_type, allowed
                )))
            }
        }
    }

    /// Gets a node type by name.
    #[must_use]
    pub fn get_node_type(&self, name: &str) -> Option<&NodeType> {
        self.node_types.iter().find(|nt| nt.name == name)
    }

    /// Gets an edge type by name.
    #[must_use]
    pub fn get_edge_type(&self, name: &str) -> Option<&EdgeType> {
        self.edge_types.iter().find(|et| et.name == name)
    }
}
