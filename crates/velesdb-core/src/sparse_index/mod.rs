//! Sparse vector types, inverted index, and search.
//!
//! This module is always compiled (no persistence dependency).
//! Persistence-related functionality is in `index::sparse::persistence`.

pub mod inverted_index;
pub mod search;
pub mod types;

#[cfg(test)]
mod batch_tests;

pub use inverted_index::SparseInvertedIndex;
pub use search::{sparse_search, sparse_search_filtered};
pub use types::{PostingEntry, ScoredDoc, SparseVector, DEFAULT_SPARSE_INDEX_NAME};
