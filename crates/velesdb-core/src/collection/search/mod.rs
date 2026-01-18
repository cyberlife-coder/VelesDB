//! Search implementation for Collection.
//!
//! This module provides all search functionality for VelesDB collections:
//! - Vector similarity search (HNSW)
//! - Full-text search (BM25)
//! - Hybrid search (vector + text with RRF fusion)
//! - Batch and multi-query search
//! - VelesQL query execution

mod batch;
mod query;
mod text;
mod vector;

// Re-export all search methods via trait implementations
// The actual impl blocks are in submodules

/// Wrapper for f32 to implement Ord for `BinaryHeap` in hybrid search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OrderedFloat(pub f32);

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
