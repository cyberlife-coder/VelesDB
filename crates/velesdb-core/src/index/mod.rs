//! Index implementations for efficient vector search.
//!
//! This module provides various index implementations for approximate
//! nearest neighbor (ANN) search and full-text search.

mod bm25;
#[cfg(test)]
mod bm25_tests;
pub mod hnsw;
mod posting_list;
#[cfg(test)]
mod posting_list_tests;
pub mod secondary;
pub mod sparse;
pub mod trigram;

pub use bm25::{Bm25Index, Bm25Params};
pub use hnsw::{HnswIndex, HnswParams, SearchQuality};
pub use secondary::{JsonValue, SecondaryIndex};
pub use sparse::{SparseInvertedIndex, SparseVector};
pub use trigram::{extract_trigrams, TrigramIndex};

use crate::distance::DistanceMetric;
use crate::scored_result::ScoredResult;
use std::cmp::Ordering;

/// Partial sort + truncate for top-k results.
///
/// When `k < items.len()`, uses `select_nth_unstable_by` to partition the
/// k smallest elements (according to `cmp`) in O(n), then sorts only those
/// k elements in O(k log k). Total: O(n + k log k) instead of O(n log n).
///
/// When `k >= items.len()`, falls back to a full sort (no-op partition).
///
/// Used by both HNSW (ascending distance) and BM25 (descending score) to
/// avoid duplicating the same algorithmic pattern.
pub(crate) fn top_k_partial_sort<T, F>(items: &mut Vec<T>, k: usize, cmp: F)
where
    F: Fn(&T, &T) -> Ordering + Copy,
{
    if items.len() > k {
        items.select_nth_unstable_by(k, cmp);
        items.truncate(k);
    }
    items.sort_by(cmp);
}

/// Trait for vector index implementations.
///
/// All index implementations must be thread-safe (Send + Sync).
///
/// # Performance Note
///
/// For bulk insertions, prefer batch methods like `HnswIndex::insert_batch_parallel()`
/// over calling [`Self::insert`] in a loop.
/// Individual inserts incur per-call lock overhead that batch methods avoid.
pub trait VectorIndex: Send + Sync {
    /// Inserts a vector into the index.
    ///
    /// # Fire-and-forget semantics
    ///
    /// This method is intentionally infallible (`-> ()`). Implementations
    /// log errors (e.g., dimension mismatch) and return silently rather
    /// than propagating failures. This design matches the trait's role as
    /// a low-level index primitive where callers validate dimensions
    /// before insertion (see `Collection::upsert`).
    ///
    /// Callers that need error feedback should use type-specific methods
    /// (e.g., `HnswIndex::insert_batch_parallel`) instead of this trait
    /// method.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the vector
    /// * `vector` - The vector data to index
    ///
    /// # Performance Warning (PERF-2)
    ///
    /// This method acquires locks for each insertion. For bulk loading, use:
    /// - `HnswIndex::insert_batch_parallel()` - Best for all batches
    ///
    /// Calling `insert()` in a loop incurs ~3x lock overhead per vector compared
    /// to batch methods which acquire locks once for the entire batch.
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
    /// A vector of [`ScoredResult`] sorted by distance/similarity.
    fn search(&self, query: &[f32], k: usize) -> Vec<ScoredResult>;

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
