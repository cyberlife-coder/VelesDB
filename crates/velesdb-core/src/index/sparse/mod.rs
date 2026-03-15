//! Sparse vector index using inverted index with segment isolation.
//!
//! Re-exports from `crate::sparse_index` (always-compiled module).
//! The persistence sub-module remains here since it depends on mmap/WAL.

#[cfg(feature = "persistence")]
pub mod persistence;
#[cfg(test)]
#[cfg(feature = "persistence")]
mod persistence_tests;

// Re-export all public types from the always-compiled sparse_index module.
pub use crate::sparse_index::inverted_index;
pub use crate::sparse_index::search;
pub use crate::sparse_index::types;

pub use crate::sparse_index::inverted_index::SparseInvertedIndex;
pub use crate::sparse_index::search::{sparse_search, sparse_search_filtered};
pub use crate::sparse_index::types::{
    PostingEntry, ScoredDoc, SparseVector, DEFAULT_SPARSE_INDEX_NAME,
};
