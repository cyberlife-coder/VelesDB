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

impl HnswInner {
    /// Searches the HNSW graph and returns raw neighbors with distances.
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
