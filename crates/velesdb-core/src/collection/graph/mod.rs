//! Graph collection module for knowledge graph storage.
//!
//! This module provides support for heterogeneous graph collections
//! that can store both vector embeddings (Points) and structured entities (Nodes)
//! connected by typed relationships (Edges).
//!
//! # Features
//!
//! - **Heterogeneous nodes**: Multiple node types with different properties
//! - **Typed edges**: Relationships with direction and properties
//! - **Schema support**: Both strict schemas and schemaless mode
//! - **Vector integration**: Nodes can have associated embeddings
//!
//! # Example
//!
//! ```rust,ignore
//! use velesdb_core::collection::graph::{GraphSchema, NodeType, EdgeType, ValueType};
//! use std::collections::HashMap;
//!
//! // Define a schema with Person and Company nodes
//! let mut person_props = HashMap::new();
//! person_props.insert("name".to_string(), ValueType::String);
//!
//! let schema = GraphSchema::new()
//!     .with_node_type(NodeType::new("Person").with_properties(person_props))
//!     .with_node_type(NodeType::new("Company"))
//!     .with_edge_type(EdgeType::new("WORKS_AT", "Person", "Company"));
//!
//! // Or use schemaless mode for flexibility
//! let flexible_schema = GraphSchema::schemaless();
//! ```

mod schema;

#[cfg(test)]
mod schema_tests;

pub use schema::{EdgeType, GraphSchema, NodeType, ValueType};
