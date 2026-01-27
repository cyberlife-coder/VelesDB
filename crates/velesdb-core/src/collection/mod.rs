//! Collection management for `VelesDB`.
//!
//! A collection is a container for vectors with associated metadata,
//! providing CRUD operations and various search capabilities.
//!
//! # Features
//!
//! - Vector storage with configurable metrics (`Cosine`, `Euclidean`, `DotProduct`)
//! - Payload storage for metadata
//! - HNSW index for fast approximate nearest neighbor search
//! - BM25 index for full-text search
//! - Hybrid search combining vector and text similarity
//! - Metadata-only collections (no vectors) for reference tables
//! - Graph collections for knowledge graph storage (nodes, edges, traversal)
//! - Async operations via `spawn_blocking` (EPIC-034/US-005)

pub mod async_ops;
pub mod auto_reindex;
mod core;
pub mod graph;
pub mod query_cost;
pub mod search;
mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod metadata_only_tests;

pub use core::{IndexInfo, TraversalResult};
pub use graph::{
    ConcurrentEdgeStore, EdgeStore, EdgeType, Element, GraphEdge, GraphNode, GraphSchema, NodeType,
    PropertyIndex, RangeIndex, ValueType,
};
pub use types::{Collection, CollectionConfig, CollectionType};
