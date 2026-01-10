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

mod core;
mod search;
mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod metadata_only_tests;

pub use types::{Collection, CollectionConfig, CollectionType};
