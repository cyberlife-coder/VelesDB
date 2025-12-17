//! Index implementations for efficient vector search.
//!
//! This module will contain HNSW and other index implementations.

// TODO: Implement HNSW index using hnsw_rs crate
// TODO: Implement flat index for small collections
// TODO: Implement IVF index for large collections

/// Placeholder for index trait.
pub trait VectorIndex: Send + Sync {
    /// Inserts a vector into the index.
    fn insert(&mut self, id: u64, vector: &[f32]);

    /// Searches for the k nearest neighbors.
    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)>;

    /// Removes a vector from the index.
    fn remove(&mut self, id: u64);

    /// Returns the number of vectors in the index.
    fn len(&self) -> usize;

    /// Returns true if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
