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

pub mod async_ops;
#[cfg(test)]
mod async_ops_tests;
pub mod auto_reindex;
mod collection_config;
mod core;
pub mod diagnostics;
pub mod graph;
mod graph_collection;
mod graph_collection_query;
mod metadata_collection;
pub mod query_cost;
pub mod search;
pub mod stats;
pub mod streaming;
pub(crate) mod text_utils;
mod types;
mod vector_collection;

mod any_collection;

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

pub use any_collection::AnyCollection;
pub use collection_config::{CollectionConfig, CURRENT_SCHEMA_VERSION};
pub use core::{IndexInfo, ScrollBatch, MAX_DIMENSION, MIN_DIMENSION};
pub use diagnostics::{CollectionDiagnostics, IndexHealth};
pub use graph::{
    ConcurrentEdgeStore, EdgeStore, EdgeType, GraphEdge, GraphNode, GraphSchema, NodeType,
    PropertyIndex, RangeIndex, TraversalConfig, TraversalPath, TraversalResult, ValueType,
};
pub use graph_collection::GraphCollection;
pub use metadata_collection::MetadataCollection;
pub(crate) use types::Collection;
pub use types::CollectionType;
pub use vector_collection::VectorCollection;
