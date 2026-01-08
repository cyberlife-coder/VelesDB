//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides high-performance approximate nearest neighbor search
//! based on the HNSW algorithm.
//!
//! # Available Implementations
//!
//! - **Default (`hnsw_rs`)**: Battle-tested, full API
//! - **Native (`native-hnsw` feature)**: 1.5x faster search, ~99% recall parity
//!
//! # Feature Flags
//!
//! - `native-hnsw`: Use native implementation instead of `hnsw_rs`
//!
//! # Module Organization
//!
//! - `params`: Index parameters and search quality profiles
//! - `native`: Native HNSW implementation with SIMD distance calculations
//! - `index`: Main `HnswIndex` implementation (default)

// ============================================================================
// Core modules (always available)
// ============================================================================
mod backend;
mod index;
mod inner;
mod mappings;
pub mod native;
mod native_index;
mod native_inner;
mod params;
mod persistence;
mod sharded_mappings;
mod sharded_vectors;
mod vector_store;

// ============================================================================
// Tests
// ============================================================================
#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod index_tests;
#[cfg(test)]
mod inner_tests;
#[cfg(test)]
mod mappings_tests;
#[cfg(test)]
mod params_tests;
#[cfg(test)]
mod parity_tests;
#[cfg(test)]
mod persistence_tests;
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

// Default: hnsw_rs-based implementation
pub use index::HnswIndex;

#[allow(unused_imports)]
pub use backend::HnswBackend;

// Native HNSW implementation (opt-in via feature flag)
#[cfg(feature = "native-hnsw")]
pub use native_index::NativeHnswIndex;
