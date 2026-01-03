//! RF-2: `HnswInner` enum extracted from `index.rs`.
//!
//! This module contains the internal HNSW wrapper enum that handles
//! different distance metrics, along with its inherent methods and
//! the `HnswBackend` trait implementation.

use hnsw_rs::prelude::*;
use std::path::Path;

/// Internal HNSW index wrapper to handle different distance metrics.
///
/// # Safety Note on `'static` Lifetime
///
/// The `'static` lifetime here is a "lifetime lie" - the actual data may be
/// borrowed from `HnswIndex::io_holder` (when loaded from disk). This is safe
/// because:
///
/// 1. The `'static` lifetime is contained within `HnswIndex` and never escapes
/// 2. `HnswIndex::Drop` ensures this enum is dropped before `io_holder`
/// 3. All access goes through `HnswIndex` which maintains the invariant
///
/// For indices created via `new()`/`with_params()`, the data is truly owned
/// and `'static` is accurate.
pub(super) enum HnswInner {
    Cosine(Hnsw<'static, f32, DistCosine>),
    Euclidean(Hnsw<'static, f32, DistL2>),
    DotProduct(Hnsw<'static, f32, DistDot>),
    /// Hamming uses L2 internally for graph construction, actual distance computed during re-ranking
    Hamming(Hnsw<'static, f32, DistL2>),
    /// Jaccard uses L2 internally for graph construction, actual similarity computed during re-ranking
    Jaccard(Hnsw<'static, f32, DistL2>),
}

// ============================================================================
// RF-1: HnswOps - Common HNSW operations consolidated into impl block
// ============================================================================
// Note: A dispatch macro cannot be used here because the enum variants have
// different generic types (DistCosine, DistL2, DistDot) which Rust cannot
// unify in a single match arm binding.

impl HnswInner {
    /// Creates a new `HnswInner` with the specified metric and parameters.
    ///
    /// RF-2.6: Factory method to eliminate duplication in `HnswIndex::with_params`.
    pub(super) fn new(
        metric: crate::distance::DistanceMetric,
        max_connections: usize,
        max_elements: usize,
        ef_construction: usize,
    ) -> Self {
        use crate::distance::DistanceMetric;

        match metric {
            DistanceMetric::Cosine => Self::Cosine(Hnsw::new(
                max_connections,
                max_elements,
                16,
                ef_construction,
                DistCosine,
            )),
            DistanceMetric::Euclidean => Self::Euclidean(Hnsw::new(
                max_connections,
                max_elements,
                16,
                ef_construction,
                DistL2,
            )),
            DistanceMetric::DotProduct => Self::DotProduct(Hnsw::new(
                max_connections,
                max_elements,
                16,
                ef_construction,
                DistDot,
            )),
            // Hamming/Jaccard use L2 for graph construction, actual distance computed during re-ranking
            DistanceMetric::Hamming => Self::Hamming(Hnsw::new(
                max_connections,
                max_elements,
                16,
                ef_construction,
                DistL2,
            )),
            DistanceMetric::Jaccard => Self::Jaccard(Hnsw::new(
                max_connections,
                max_elements,
                16,
                ef_construction,
                DistL2,
            )),
        }
    }

    /// Searches the HNSW graph and returns raw neighbors with distances.
    #[inline]
    pub(super) fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<Neighbour> {
        match self {
            Self::Cosine(hnsw) => hnsw.search(query, k, ef_search),
            Self::Euclidean(hnsw) | Self::Hamming(hnsw) | Self::Jaccard(hnsw) => {
                hnsw.search(query, k, ef_search)
            }
            Self::DotProduct(hnsw) => hnsw.search(query, k, ef_search),
        }
    }

    /// Inserts a single vector into the HNSW graph.
    pub(super) fn insert(&self, data: (&[f32], usize)) {
        match self {
            Self::Cosine(hnsw) => hnsw.insert(data),
            Self::Euclidean(hnsw) | Self::Hamming(hnsw) | Self::Jaccard(hnsw) => hnsw.insert(data),
            Self::DotProduct(hnsw) => hnsw.insert(data),
        }
    }

    /// Parallel batch insert into the HNSW graph.
    pub(super) fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) {
        match self {
            Self::Cosine(hnsw) => hnsw.parallel_insert(data),
            Self::Euclidean(hnsw) | Self::Hamming(hnsw) | Self::Jaccard(hnsw) => {
                hnsw.parallel_insert(data);
            }
            Self::DotProduct(hnsw) => hnsw.parallel_insert(data),
        }
    }

    /// Sets the index to searching mode after bulk insertions.
    pub(super) fn set_searching_mode(&mut self, mode: bool) {
        match self {
            Self::Cosine(hnsw) => hnsw.set_searching_mode(mode),
            Self::Euclidean(hnsw) | Self::Hamming(hnsw) | Self::Jaccard(hnsw) => {
                hnsw.set_searching_mode(mode);
            }
            Self::DotProduct(hnsw) => hnsw.set_searching_mode(mode),
        }
    }

    /// Dumps the HNSW graph to files for persistence.
    pub(super) fn file_dump(&self, path: &Path, basename: &str) -> Result<(), std::io::Error> {
        match self {
            Self::Cosine(hnsw) => hnsw
                .file_dump(path, basename)
                .map(|_| ())
                .map_err(std::io::Error::other),
            Self::Euclidean(hnsw) | Self::Hamming(hnsw) | Self::Jaccard(hnsw) => hnsw
                .file_dump(path, basename)
                .map(|_| ())
                .map_err(std::io::Error::other),
            Self::DotProduct(hnsw) => hnsw
                .file_dump(path, basename)
                .map(|_| ())
                .map_err(std::io::Error::other),
        }
    }

    /// Transforms raw HNSW distance to the appropriate score based on metric type.
    ///
    /// - **Cosine**: `(1.0 - distance).clamp(0.0, 1.0)` (similarity in `[0,1]`)
    /// - **Euclidean**/**Hamming**/**Jaccard**: raw distance (lower is better)
    /// - **`DotProduct`**: `-distance` (`hnsw_rs` stores negated dot product)
    #[inline]
    pub(super) fn transform_score(&self, raw_distance: f32) -> f32 {
        match self {
            Self::Cosine(_) => (1.0 - raw_distance).clamp(0.0, 1.0),
            Self::Euclidean(_) | Self::Hamming(_) | Self::Jaccard(_) => raw_distance,
            Self::DotProduct(_) => -raw_distance,
        }
    }
}

// ============================================================================
// FT-1: HnswBackend trait implementation
// ============================================================================

impl super::backend::HnswBackend for HnswInner {
    #[inline]
    fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<Neighbour> {
        HnswInner::search(self, query, k, ef_search)
    }

    #[inline]
    fn insert(&self, data: (&[f32], usize)) {
        HnswInner::insert(self, data);
    }

    #[inline]
    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) {
        HnswInner::parallel_insert(self, data);
    }

    #[inline]
    fn set_searching_mode(&mut self, mode: bool) {
        HnswInner::set_searching_mode(self, mode);
    }

    #[inline]
    fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()> {
        HnswInner::file_dump(self, path, basename)
    }

    #[inline]
    fn transform_score(&self, raw_distance: f32) -> f32 {
        HnswInner::transform_score(self, raw_distance)
    }
}

// ============================================================================
// Tests (must be at end of file per clippy::items_after_test_module)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test search works for all distance metrics
    #[test]
    fn test_hnsw_inner_search_all_metrics() {
        let indices = [
            HnswInner::Cosine(Hnsw::new(16, 100, 16, 4, DistCosine)),
            HnswInner::Euclidean(Hnsw::new(16, 100, 16, 4, DistL2)),
            HnswInner::DotProduct(Hnsw::new(16, 100, 16, 4, DistDot)),
        ];

        for index in &indices {
            let query = vec![0.5_f32; 4];
            let results = index.search(&query, 3, 32);
            assert!(results.is_empty());
        }
    }

    /// Test insert works for `HnswInner`
    #[test]
    fn test_hnsw_inner_insert() {
        let index = HnswInner::Cosine(Hnsw::new(16, 100, 16, 4, DistCosine));
        let vector = vec![0.1_f32; 4];
        index.insert((&vector, 0));
        let results = index.search(&vector, 1, 32);
        assert_eq!(results.len(), 1);
    }

    /// Test `transform_score` for different metrics
    #[test]
    fn test_hnsw_inner_transform_score() {
        let cosine = HnswInner::Cosine(Hnsw::new(16, 100, 16, 4, DistCosine));
        let euclidean = HnswInner::Euclidean(Hnsw::new(16, 100, 16, 4, DistL2));
        let dot = HnswInner::DotProduct(Hnsw::new(16, 100, 16, 4, DistDot));

        assert!((cosine.transform_score(0.5) - 0.5).abs() < 0.001);
        assert!((euclidean.transform_score(0.5) - 0.5).abs() < 0.001);
        assert!((dot.transform_score(0.5) - (-0.5)).abs() < 0.001);
    }
}
