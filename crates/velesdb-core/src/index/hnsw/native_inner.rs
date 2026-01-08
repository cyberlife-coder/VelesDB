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
}

// ============================================================================
// Send + Sync for thread safety
// ============================================================================

unsafe impl Send for NativeHnswInner {}
unsafe impl Sync for NativeHnswInner {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_native_inner_new_all_metrics() {
        for metric in [
            DistanceMetric::Cosine,
            DistanceMetric::Euclidean,
            DistanceMetric::DotProduct,
            DistanceMetric::Hamming,
            DistanceMetric::Jaccard,
        ] {
            let inner = NativeHnswInner::new(metric, 16, 100, 100);
            assert_eq!(inner.metric(), metric);
            assert!(inner.is_empty());
        }
    }

    #[test]
    fn test_native_inner_insert_and_search() {
        let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);

        // Insert vectors
        for i in 0..20 {
            let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
            inner.insert((&vec, i));
        }

        assert_eq!(inner.len(), 20);

        // Search
        let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
        let results = inner.search(&query, 5, 50);

        assert!(!results.is_empty());
        assert!(results.len() <= 5);
        // First result should be closest to query (which is vector 0)
        assert_eq!(results[0].d_id, 0);
    }

    #[test]
    fn test_native_inner_transform_score_cosine() {
        let inner = NativeHnswInner::new(DistanceMetric::Cosine, 16, 100, 100);
        assert!((inner.transform_score(0.3) - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn test_native_inner_transform_score_euclidean() {
        let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
        assert!((inner.transform_score(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_native_inner_transform_score_dot_product() {
        let inner = NativeHnswInner::new(DistanceMetric::DotProduct, 16, 100, 100);
        assert!((inner.transform_score(0.5) - (-0.5)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_native_inner_persistence_roundtrip() {
        let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);

        // Insert vectors
        let vectors: Vec<Vec<f32>> = (0..30)
            .map(|i| (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect())
            .collect();

        for (i, v) in vectors.iter().enumerate() {
            inner.insert((v, i));
        }

        // Dump
        let dir = tempdir().unwrap();
        inner.file_dump(dir.path(), "native_test").unwrap();

        // Load
        let loaded =
            NativeHnswInner::file_load(dir.path(), "native_test", DistanceMetric::Euclidean)
                .unwrap();

        assert_eq!(loaded.len(), 30);
        assert_eq!(loaded.metric(), DistanceMetric::Euclidean);

        // Search should return same results
        let query = vectors[0].clone();
        let results_orig = inner.search(&query, 5, 50);
        let results_loaded = loaded.search(&query, 5, 50);

        assert_eq!(results_orig.len(), results_loaded.len());
        if !results_orig.is_empty() {
            assert_eq!(results_orig[0].d_id, results_loaded[0].d_id);
        }
    }

    #[test]
    fn test_native_inner_parallel_insert() {
        let inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 200);

        let vectors: Vec<Vec<f32>> = (0..150).map(|i| vec![i as f32 * 0.01; 32]).collect();
        let data: Vec<(&Vec<f32>, usize)> =
            vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();

        inner.parallel_insert(&data);

        assert_eq!(inner.len(), 150);
    }

    #[test]
    fn test_native_inner_set_searching_mode_no_panic() {
        let mut inner = NativeHnswInner::new(DistanceMetric::Euclidean, 16, 100, 100);
        inner.set_searching_mode(true);
        inner.set_searching_mode(false);
    }
}
