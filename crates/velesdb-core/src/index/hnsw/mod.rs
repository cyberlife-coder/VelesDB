//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides high-performance approximate nearest neighbor search
//! based on the HNSW algorithm.
//!
//! # Native Implementation (v1.0+)
//!
//! `VelesDB` uses a custom native HNSW implementation that is:
//! - **1.2x faster search** than external libraries
//! - **1.07x faster parallel insert**
//! - **~99% recall parity** with no accuracy loss
//!
//! # Module Organization
//!
//! - `params`: Index parameters and search quality profiles
//! - `native`: Core HNSW graph with SIMD distance calculations
//! - `index`: Main `HnswIndex` API

// ============================================================================
// Core modules
// ============================================================================
pub(crate) mod auto_ef;
pub(crate) mod direct_writer;
mod index;
pub mod native;
pub mod native_index;
mod native_index_io;
#[cfg(test)]
mod native_index_tests;
mod native_inner;
mod params;
pub(crate) mod persistence;
mod sharded_mappings;
mod sharded_vectors;
pub(crate) mod upsert;
// ============================================================================
// Tests
// ============================================================================
#[cfg(test)]
mod auto_ef_tests;
#[cfg(test)]
mod direct_writer_tests;
#[cfg(test)]
mod gpu_rerank_tests;
#[cfg(test)]
mod index_tests;
#[cfg(test)]
mod params_tests;
#[cfg(test)]
mod persistence_atomicity_tests;
#[cfg(test)]
mod sharded_mappings_tests;
#[cfg(test)]
mod sharded_vectors_tests;
#[cfg(test)]
mod upsert_tests;

// ============================================================================
// Public API
// ============================================================================
pub use params::{HnswParams, SearchQuality};

/// Main HNSW index for vector search operations.
pub use index::HnswIndex;

/// Native HNSW index with direct access to underlying graph.
pub use native_index::NativeHnswIndex;
