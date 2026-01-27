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
//! - **Unified storage**: Points and Nodes in the same ID space
//!
//! # Example
//!
//! ```rust,ignore
//! use velesdb_core::collection::graph::{GraphSchema, NodeType, EdgeType, ValueType, GraphNode, Element};
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
//! // Create a graph node
//! let node = GraphNode::new(1, "Person")
//!     .with_vector(vec![0.1, 0.2, 0.3]);
//!
//! // Or use schemaless mode for flexibility
//! let flexible_schema = GraphSchema::schemaless();
//! ```

mod edge;
mod edge_concurrent;
mod label_table;
mod metrics;
mod node;
mod property_index;
mod range_index;
mod schema;
mod streaming;
#[cfg(test)]
mod streaming_tests;
mod traversal;
#[cfg(test)]
mod traversal_tests;

#[cfg(test)]
mod edge_concurrent_tests;
#[cfg(test)]
mod edge_tests;
#[cfg(test)]
mod node_tests;
#[cfg(test)]
mod property_index_tests;
#[cfg(test)]
mod range_index_tests;
#[cfg(test)]
mod schema_tests;

pub use edge::{EdgeStore, GraphEdge};
pub use edge_concurrent::ConcurrentEdgeStore;
pub use label_table::{LabelId, LabelTable};
pub use metrics::{GraphMetrics, LatencyHistogram};
pub use node::{Element, GraphNode};
pub use property_index::PropertyIndex;
pub use range_index::{OrderedValue, RangeIndex};
pub use schema::{EdgeType, GraphSchema, NodeType, ValueType};
pub use streaming::{bfs_stream, BfsIterator, StreamingConfig};
pub use traversal::{TraversalConfig, TraversalResult, DEFAULT_MAX_DEPTH};
