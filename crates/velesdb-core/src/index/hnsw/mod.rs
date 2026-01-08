//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides high-performance approximate nearest neighbor search
//! based on the HNSW algorithm.
//!
//! # Available Implementations (v0.8.12+)
//!
//! - **Native (default)**: 1.2x faster search, ~99% recall parity
//! - **Legacy (`legacy-hnsw` feature)**: Uses `hnsw_rs` library for compatibility
//!
//! # Feature Flags
//!
//! - `native-hnsw` (default): Use native HNSW implementation
//! - `legacy-hnsw`: Fall back to `hnsw_rs` library
//!
//! # Module Organization
//!
//! - `params`: Index parameters and search quality profiles
//! - `native`: Native HNSW implementation with SIMD distance calculations
//! - `index`: Main `HnswIndex` implementation

// ============================================================================
// Core modules (always available)
// ============================================================================
mod backend;
mod index;
mod mappings;
pub mod native;
mod native_index;
mod native_inner;
mod params;
mod sharded_mappings;
mod sharded_vectors;
mod vector_store;

// ============================================================================
// Legacy hnsw_rs modules (only with legacy-hnsw feature)
// ============================================================================
#[cfg(feature = "legacy-hnsw")]
mod inner;
#[cfg(feature = "legacy-hnsw")]
mod persistence;

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

// Legacy tests (only with legacy-hnsw feature)
#[cfg(all(test, feature = "legacy-hnsw"))]
mod backend_tests;
#[cfg(all(test, feature = "legacy-hnsw"))]
mod inner_tests;
#[cfg(all(test, feature = "legacy-hnsw"))]
mod parity_tests;
#[cfg(all(test, feature = "legacy-hnsw"))]
mod persistence_tests;

// ============================================================================
// Public API
// ============================================================================
pub use params::{HnswParams, SearchQuality};

// Main HnswIndex - works with both native and legacy backends
pub use index::HnswIndex;

#[allow(unused_imports)]
pub use backend::HnswBackend;

// Native HNSW implementation (always available)
pub use native_index::NativeHnswIndex;
