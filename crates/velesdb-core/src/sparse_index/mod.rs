//! Sparse vector types, inverted index, and search.
//!
//! This module is always compiled (no persistence dependency).
//! Persistence-related functionality is in `index::sparse::persistence`.
#![allow(dead_code)] // FrozenSegment methods used by persistence layer via index::sparse re-export.

pub mod inverted_index;
pub mod search;
pub mod types;

pub use inverted_index::SparseInvertedIndex;
pub use search::{sparse_search, sparse_search_filtered};
pub use types::{PostingEntry, ScoredDoc, SparseVector};
