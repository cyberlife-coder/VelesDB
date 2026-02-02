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
mod backend;
mod index;
mod mappings;
pub mod native;
pub mod native_index;
#[cfg(test)]
mod native_index_tests;
mod native_inner;
mod params;
mod sharded_mappings;
mod sharded_vectors;
mod vector_store;

// ============================================================================
// Tests
// ============================================================================
#[cfg(test)]
mod index_tests;
#[cfg(test)]
mod mappings_tests;
#[cfg(test)]
mod params_tests;
#[cfg(test)]
mod sharded_mappings_tests;
#[cfg(test)]
mod sharded_vectors_tests;
#[cfg(test)]
mod vector_store_tests;

// ============================================================================
// Public API
// ============================================================================
pub use params::{HnswParams, SearchQuality};

/// Main HNSW index for vector search operations.
pub use index::HnswIndex;

/// HNSW backend trait for custom implementations.
#[allow(unused_imports)]
pub use backend::HnswBackend;

/// Native HNSW index with direct access to underlying graph.
pub use native_index::NativeHnswIndex;
