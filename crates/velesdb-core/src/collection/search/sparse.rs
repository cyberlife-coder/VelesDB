//! Hybrid dense+sparse search over a raw `SparseVector`, for Collection.
//!
//! The VelesQL path goes through `execute_sparse_search` /
//! `execute_hybrid_search_with_strategy` with AST nodes; this is the entry
//! point that takes the query vectors directly. Its only caller is
//! `VectorCollection::hybrid_sparse_search_filtered`, which every SDK binding
//! (Python, TypeScript, Mobile, CLI, Tauri) reaches.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::fusion::FusionStrategy;
use crate::point::SearchResult;
use crate::sparse_index::SparseVector;

impl Collection {
    /// Hybrid dense+sparse search with an optional filter — the body of
    /// `VectorCollection::hybrid_sparse_search_filtered`, so every hybrid
    /// entry point fuses the same way.
    ///
    /// # Errors
    ///
    /// Returns an error if either branch fails (a dense dimension mismatch,
    /// an unknown sparse index) or fusion fails.
    pub(crate) fn hybrid_sparse_search_inner(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        k: usize,
        strategy: &FusionStrategy,
        filter: Option<&crate::filter::Filter>,
        index_name: &str,
    ) -> Result<Vec<SearchResult>> {
        let candidate_k = k.saturating_mul(2).max(k.saturating_add(10));

        let (dense_results, sparse_results) = self.execute_both_branches(
            dense_vector,
            sparse_query,
            index_name,
            candidate_k,
            filter,
        )?;

        self.fuse_dense_sparse(dense_results, &sparse_results, k, strategy)
    }
}
