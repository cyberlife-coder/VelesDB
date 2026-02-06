//! Native HNSW inner implementation - replaces `hnsw_rs` dependency.
//!
//! This module provides `NativeHnswInner`, a drop-in replacement for `HnswInner`
//! that uses our native HNSW implementation instead of the `hnsw_rs` crate.

// Temporarily allow dead_code until integration into HnswIndex
#![allow(dead_code)]
#![allow(clippy::cast_precision_loss)]

use super::native::{NativeHnsw, NativeNeighbour, SimdDistance};
use crate::distance::DistanceMetric;
use std::path::Path;

/// Native HNSW index wrapper to handle different distance metrics.
///
/// This is the native equivalent of `HnswInner`, using our own HNSW implementation
/// instead of `hnsw_rs`. It provides the same API for seamless integration.
pub struct NativeHnswInner {
    /// The underlying native HNSW index
    inner: NativeHnsw<SimdDistance>,
    /// The distance metric used
    metric: DistanceMetric,
}

impl NativeHnswInner {
    /// Creates a new `NativeHnswInner` with the specified metric and parameters.
    #[must_use]
    pub fn new(
        metric: DistanceMetric,
        max_connections: usize,
        max_elements: usize,
        ef_construction: usize,
    ) -> Self {
        let distance = SimdDistance::new(metric);
        let inner = NativeHnsw::new(distance, max_connections, ef_construction, max_elements);

        Self { inner, metric }
    }

    /// Searches the HNSW graph and returns raw neighbors with distances.
    #[inline]
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<NativeNeighbour> {
        self.inner.search_neighbours(query, k, ef_search)
    }

    /// Inserts a single vector into the HNSW graph.
    ///
    /// Note: Unlike `hnsw_rs`, our native implementation auto-assigns IDs.
    /// The returned node ID should be stored in the mappings.
    pub fn insert(&self, data: (&[f32], usize)) -> usize {
        self.inner.insert(data.0.to_vec())
    }

    /// Parallel batch insert into the HNSW graph.
    pub fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) {
        self.inner.parallel_insert(data);
    }

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// Note: This is a no-op for our native implementation.
    pub fn set_searching_mode(&mut self, mode: bool) {
        self.inner.set_searching_mode(mode);
    }

    /// Dumps the HNSW graph to files for persistence.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail.
    pub fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()> {
        self.inner.file_dump(path, basename)
    }

    /// Loads the HNSW graph from files.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail or data is corrupted.
    pub fn file_load(path: &Path, basename: &str, metric: DistanceMetric) -> std::io::Result<Self> {
        let distance = SimdDistance::new(metric);
        let inner = NativeHnsw::file_load(path, basename, distance)?;

        Ok(Self { inner, metric })
    }

    /// Transforms raw HNSW distance to the appropriate score based on metric type.
    ///
    /// - **Cosine**: `(1.0 - distance).clamp(0.0, 1.0)` (similarity in `[0,1]`)
    /// - **Euclidean**/**Hamming**/**Jaccard**: raw distance (lower is better)
    /// - **`DotProduct`**: `-distance` (negated for consistency)
    #[inline]
    #[must_use]
    pub fn transform_score(&self, raw_distance: f32) -> f32 {
        self.inner.transform_score(raw_distance)
    }

    /// Returns the number of elements in the index.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the index is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the distance metric used by this index.
    #[inline]
    #[must_use]
    pub fn metric(&self) -> DistanceMetric {
        self.metric
    }

    /// Computes the distance between two vectors using the index's distance metric.
    ///
    /// This is useful for brute-force search where we need to compute distances
    /// outside of the HNSW graph traversal.
    #[inline]
    #[must_use]
    pub fn compute_distance(&self, a: &[f32], b: &[f32]) -> f32 {
        self.inner.compute_distance(a, b)
    }
}

// ============================================================================
// Send + Sync for thread safety
// ============================================================================

// SAFETY: `NativeHnswInner` is `Send` because ownership transfer preserves invariants.
// - Condition 1: Internal mutability is synchronized via `parking_lot::RwLock`/atomics.
// - Condition 2: No thread-affine resources are stored in the wrapper.
// Reason: Moving the index wrapper between threads is sound.
unsafe impl Send for NativeHnswInner {}
// SAFETY: `NativeHnswInner` is `Sync` because shared references are concurrency-safe.
// - Condition 1: Concurrent access to mutable graph state is lock/atomic protected.
// - Condition 2: Exposed APIs do not bypass synchronization primitives.
// Reason: `&NativeHnswInner` can be shared safely across threads.
unsafe impl Sync for NativeHnswInner {}

// ============================================================================
// Tests moved to native_inner_tests.rs per project rules
