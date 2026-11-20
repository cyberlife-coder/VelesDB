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
#[cfg(test)]
mod expiry_tests;
#[cfg(test)]
mod hybrid_branch_tests;
mod live;
pub(crate) mod pending;
#[cfg(test)]
mod pending_tests;
pub mod query;
#[cfg(test)]
mod query_validation_tests;
mod query_vector_check;
pub(crate) mod ranked;
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

pub(crate) use vector::tag_vector_component_scores;
