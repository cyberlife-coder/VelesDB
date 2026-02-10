//! In-memory graph module (no persistence dependencies).
//!
//! Provides graph types, edge storage, and traversal algorithms that work
//! without the `persistence` feature. This enables WASM and other non-persistence
//! consumers to use graph data structures from velesdb-core.
//!
//! # Example
//!
//! ```rust
//! use velesdb_core::graph::{GraphNode, GraphEdge, InMemoryEdgeStore};
//! use velesdb_core::graph::traversal::{bfs, TraversalConfig};
//!
//! let mut store = InMemoryEdgeStore::new();
//! store.add_node(GraphNode::new(1, "Person")).unwrap();
//! store.add_node(GraphNode::new(2, "Company")).unwrap();
//! store.add_edge(GraphEdge::new(100, 1, 2, "WORKS_AT").unwrap()).unwrap();
//!
//! let results = bfs(&store, 1, &TraversalConfig::new(3, 100));
//! assert_eq!(results.len(), 1);
//! assert_eq!(results[0].node_id, 2);
//! ```

mod edge_store;
pub mod traversal;
mod types;

#[cfg(test)]
mod edge_store_tests;
#[cfg(test)]
mod traversal_tests;
#[cfg(test)]
mod types_tests;

pub use edge_store::InMemoryEdgeStore;
pub use traversal::{GraphTraversal, TraversalConfig, TraversalStep};
pub use types::{GraphEdge, GraphNode};
