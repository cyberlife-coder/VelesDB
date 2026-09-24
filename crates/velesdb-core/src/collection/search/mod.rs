//! Search implementation for Collection.
//!
//! This module provides all search functionality for VelesDB collections:
//! - Vector similarity search (HNSW)
//! - Full-text search (BM25)
//! - Hybrid search (vector + text with RRF fusion)
//! - Batch and multi-query search
//! - VelesQL query execution

mod batch;
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod distance_semantics_tests;
pub mod query;
#[cfg(test)]
mod query_validation_tests;
pub(crate) mod resolve;
#[cfg(test)]
mod similarity_exec_tests;
mod sparse;
#[cfg(test)]
mod sparse_tests;
mod text;
mod text_fusion;
#[cfg(test)]
mod text_tests;
mod vector;
#[cfg(test)]
mod vector_dispatch_parity_tests;
mod vector_filter;
#[cfg(test)]
mod vector_filter_tests;
#[cfg(test)]
mod vector_tests;

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
        // The fused-score order `sort_fused_results` uses, so the top-k heap
        // and a full sort agree: `total_cmp` with `-0.0` tied to `0.0`. A
        // positive NaN is the greatest score: the min-heap
        // (`Reverse<(OrderedFloat, _)>`) never evicts it, so it is kept and
        // sorts first, as `sort_fused_results` documents.
        crate::fusion::fused_score_cmp(self.0, other.0)
    }
}
