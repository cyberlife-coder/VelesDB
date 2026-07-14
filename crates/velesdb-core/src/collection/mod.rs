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

// Persistence-free statistics/cost leaves consumed by the `VelesQL` query planner
// (P1.4). These compile without the `persistence` feature so that
// `velesql::{planner, explain, cost_estimator, query_stats}` can be ungated too.
// The remaining collection submodules are storage/index-coupled and stay gated.
pub mod query_cost;
pub mod stats;

#[cfg(feature = "persistence")]
pub mod auto_reindex;
#[cfg(feature = "persistence")]
mod collection_config;
#[cfg(feature = "persistence")]
pub(crate) mod config_serde;
#[cfg(feature = "persistence")]
mod core;
#[cfg(feature = "persistence")]
pub mod diagnostics;
#[cfg(feature = "persistence")]
pub(crate) mod expiry;
#[cfg(feature = "persistence")]
pub mod graph;
#[cfg(feature = "persistence")]
mod graph_collection;
#[cfg(feature = "persistence")]
mod graph_collection_query;
#[cfg(feature = "persistence")]
mod metadata_collection;
#[cfg(feature = "persistence")]
pub(crate) mod order_by_advisor;
#[cfg(feature = "persistence")]
pub(crate) mod payload_mirror;
#[cfg(feature = "persistence")]
pub(crate) mod payload_size;
#[cfg(feature = "persistence")]
pub mod search;
#[cfg(feature = "persistence")]
pub mod streaming;
#[cfg(feature = "persistence")]
pub(crate) mod text_utils;
#[cfg(feature = "persistence")]
mod types;
#[cfg(feature = "persistence")]
mod vector_collection;

#[cfg(feature = "persistence")]
mod any_collection;

#[cfg(all(test, feature = "persistence"))]
mod tests;

#[cfg(all(test, feature = "persistence"))]
mod metadata_collection_tests;

#[cfg(all(test, feature = "persistence"))]
mod metadata_only_tests;

#[cfg(all(test, feature = "persistence"))]
mod guardrails_integration_tests;

#[cfg(all(test, feature = "persistence"))]
mod e2e_integration_tests;

#[cfg(all(test, feature = "persistence"))]
mod set_operations_execution_tests;

#[cfg(feature = "persistence")]
pub use any_collection::AnyCollection;
#[cfg(feature = "persistence")]
pub use collection_config::{CollectionConfig, CURRENT_SCHEMA_VERSION};
#[cfg(feature = "persistence")]
pub use core::{IndexInfo, ScrollBatch, MAX_DIMENSION, MIN_DIMENSION};
#[cfg(feature = "persistence")]
pub use diagnostics::{CollectionDiagnostics, IndexHealth};
#[cfg(feature = "persistence")]
pub use expiry::EXPIRES_AT_KEY;
#[cfg(feature = "persistence")]
pub use graph::{
    ConcurrentEdgeStore, EdgeStore, EdgeType, GraphEdge, GraphNode, GraphSchema, NodeType,
    PropertyIndex, RangeIndex, TraversalConfig, TraversalPath, TraversalResult, ValueType,
};
#[cfg(feature = "persistence")]
pub use graph_collection::GraphCollection;
#[cfg(feature = "persistence")]
pub use metadata_collection::MetadataCollection;
#[cfg(feature = "persistence")]
pub use order_by_advisor::{OrderByIndexState, OrderByIndexSuggestion};
#[cfg(feature = "persistence")]
pub(crate) use types::Collection;
#[cfg(feature = "persistence")]
pub use types::CollectionType;
#[cfg(feature = "persistence")]
pub(crate) use types::RuntimeLimits;
#[cfg(feature = "persistence")]
pub use vector_collection::VectorCollection;
