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
#![allow(clippy::doc_markdown)] // Collection docs contain many API/algorithm identifiers.
#![allow(deprecated)] // Collection is deprecated but used internally throughout this module tree.

pub mod async_ops;
#[cfg(test)]
mod async_ops_tests;
pub mod auto_reindex;
mod core;
pub mod diagnostics;
pub mod graph;
mod graph_collection;
mod metadata_collection;
pub mod query_cost;
pub mod search;
pub mod stats;
pub mod streaming;
pub(crate) mod text_utils;
mod types;
mod vector_collection;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod metadata_collection_tests;

#[cfg(test)]
mod metadata_only_tests;

#[cfg(test)]
mod guardrails_integration_tests;

#[cfg(test)]
mod e2e_integration_tests;

#[cfg(test)]
mod set_operations_execution_tests;

pub use core::{IndexInfo, MAX_DIMENSION, MIN_DIMENSION};
pub use diagnostics::{CollectionDiagnostics, IndexHealth};
pub use graph::{
    ConcurrentEdgeStore, EdgeStore, EdgeType, Element, GraphEdge, GraphNode, GraphSchema, NodeType,
    PropertyIndex, RangeIndex, TraversalResult, ValueType,
};
pub use graph_collection::GraphCollection;
pub use metadata_collection::MetadataCollection;
pub use types::{Collection, CollectionConfig, CollectionType};
pub use vector_collection::VectorCollection;
