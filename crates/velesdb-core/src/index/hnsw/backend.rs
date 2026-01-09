//! HNSW Backend Trait Abstraction
//!
//! This module defines the `HnswBackend` trait that abstracts HNSW graph operations,
//! enabling:
//! - Mock backends for testing
//! - Custom backend implementations
//!
//! # Native Implementation (v1.0+)
//!
//! Uses `NativeNeighbour` from the native HNSW implementation.

use std::path::Path;

use super::native::NativeNeighbour as Neighbour;

/// Trait for HNSW backend operations.
///
/// This trait abstracts the core HNSW graph operations, allowing `HnswIndex`
/// to work with different backend implementations.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to support concurrent access
/// patterns used by `HnswIndex`.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::index::hnsw::HnswBackend;
///
/// fn search_with_backend<B: HnswBackend>(backend: &B, query: &[f32]) -> Vec<Neighbour> {
///     backend.search(query, 10, 100)
/// }
/// ```
// FT-1: Trait prepared for RF-2 (index.rs split). Will be used in production after RF-2.
#[allow(dead_code)]
pub trait HnswBackend: Send + Sync {
    /// Searches the HNSW graph and returns raw neighbors with distances.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `ef_search` - Search expansion factor (higher = more accurate, slower)
    ///
    /// # Returns
    ///
    /// Vector of `Neighbour` structs containing (distance, index) pairs.
    fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<Neighbour>;

    /// Inserts a single vector into the HNSW graph.
    ///
    /// # Arguments
    ///
    /// * `data` - Tuple of (vector slice, internal index)
    fn insert(&self, data: (&[f32], usize));

    /// Batch parallel insert into the HNSW graph.
    ///
    /// Uses rayon internally for parallel insertion.
    ///
    /// # Arguments
    ///
    /// * `data` - Slice of (vector reference, internal index) pairs
    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]);

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// This optimizes the graph structure for search queries.
    ///
    /// # Arguments
    ///
    /// * `mode` - `true` to enable searching mode, `false` to disable
    fn set_searching_mode(&mut self, mode: bool);

    /// Dumps the HNSW graph to files for persistence.
    ///
    /// # Arguments
    ///
    /// * `path` - Directory path for output files
    /// * `basename` - Base name for output files
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail.
    fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()>;

    /// Transforms raw HNSW distance to the appropriate score based on metric type.
    ///
    /// Different distance metrics require different score transformations:
    /// - **Cosine**: `(1.0 - distance).clamp(0.0, 1.0)` (similarity in `[0,1]`)
    /// - **Euclidean**/**Hamming**/**Jaccard**: raw distance (lower is better)
    /// - **`DotProduct`**: `-distance` (`hnsw_rs` stores negated dot product)
    ///
    /// # Arguments
    ///
    /// * `raw_distance` - The raw distance value from HNSW search
    ///
    /// # Returns
    ///
    /// Transformed score appropriate for the metric type.
    fn transform_score(&self, raw_distance: f32) -> f32;
}

// Note: impl HnswBackend for HnswInner is in index.rs to avoid method name
// conflicts with the inherent impl methods.
