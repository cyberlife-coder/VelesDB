//! Index implementations for efficient vector search.
//!
//! This module provides various index implementations for approximate
//! nearest neighbor (ANN) search and full-text search.

mod bm25;
mod hnsw;

pub use bm25::{Bm25Index, Bm25Params};
pub use hnsw::{HnswIndex, HnswParams, SearchQuality};

use crate::distance::DistanceMetric;

/// Trait for vector index implementations.
///
/// All index implementations must be thread-safe (Send + Sync).
pub trait VectorIndex: Send + Sync {
    /// Inserts a vector into the index.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the vector
    /// * `vector` - The vector data to index
    fn insert(&self, id: u64, vector: &[f32]);

    /// Searches for the k nearest neighbors of the query vector.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    ///
    /// # Returns
    ///
    /// A vector of (id, distance) tuples, sorted by distance.
    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)>;

    /// Removes a vector from the index.
    ///
    /// # Arguments
    ///
    /// * `id` - The identifier of the vector to remove
    ///
    /// # Returns
    ///
    /// `true` if the vector was found and removed, `false` otherwise.
    fn remove(&self, id: u64) -> bool;

    /// Returns the number of vectors in the index.
    fn len(&self) -> usize;

    /// Returns `true` if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the dimension of vectors in this index.
    fn dimension(&self) -> usize;

    /// Returns the distance metric used by this index.
    fn metric(&self) -> DistanceMetric;
}
