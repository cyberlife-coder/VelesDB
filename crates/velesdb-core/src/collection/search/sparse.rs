//! Public sparse and hybrid dense+sparse search methods for Collection.
//!
//! These methods provide a simpler API than the VelesQL-based internal
//! `execute_sparse_search` / `execute_hybrid_search_with_strategy` methods,
//! accepting raw `SparseVector` directly instead of VelesQL AST nodes.
//! Designed for SDK wiring (Python, TypeScript, Mobile).

use super::resolve;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::fusion::FusionStrategy;
use crate::point::SearchResult;
use crate::sparse_index::{search::sparse_search, SparseVector, DEFAULT_SPARSE_INDEX_NAME};

impl Collection {
    /// Sparse-only search on the default sparse index.
    ///
    /// # Errors
    ///
    /// Returns an error if the default sparse index does not exist.
    pub fn sparse_search_default(
        &self,
        query: &SparseVector,
        k: usize,
    ) -> Result<Vec<SearchResult>> {
        self.sparse_search_named(query, k, DEFAULT_SPARSE_INDEX_NAME)
    }

    /// Sparse-only search on a named sparse index.
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist.
    pub fn sparse_search_named(
        &self,
        query: &SparseVector,
        k: usize,
        index_name: &str,
    ) -> Result<Vec<SearchResult>> {
        let indexes = self.sparse_indexes.read();
        let index = indexes
            .get(index_name)
            .ok_or_else(|| resolve::sparse_index_not_found(index_name))?;
        let results = sparse_search(index, query, k);
        // Explicit drop: `resolve_sparse_results` acquires the payload_storage read-lock,
        // which is ordered after sparse_indexes in the Collection lock hierarchy.
        // Releasing sparse_indexes here before entering resolve_sparse_results prevents
        // a potential lock-ordering violation if the call path ever reacquires sparse_indexes.
        drop(indexes);
        Ok(self.resolve_sparse_results(&results, k))
    }

    /// Hybrid dense+sparse search with RRF fusion on the default sparse index.
    ///
    /// Runs both dense (HNSW) and sparse branches, then fuses using the
    /// provided strategy (typically RRF with k=60).
    ///
    /// # Errors
    ///
    /// Returns an error if the sparse index does not exist or fusion fails.
    pub fn hybrid_sparse_search(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        k: usize,
        strategy: &FusionStrategy,
    ) -> Result<Vec<SearchResult>> {
        self.hybrid_sparse_search_inner(
            dense_vector,
            sparse_query,
            k,
            strategy,
            None,
            DEFAULT_SPARSE_INDEX_NAME,
        )
    }

    /// Hybrid dense+sparse search with metadata filtering.
    ///
    /// Same as [`hybrid_sparse_search`](Self::hybrid_sparse_search) but applies
    /// a metadata filter to the sparse branch during candidate retrieval.
    ///
    /// # Errors
    ///
    /// Returns an error if the sparse index does not exist or fusion fails.
    pub fn hybrid_sparse_search_with_filter(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        k: usize,
        strategy: &FusionStrategy,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        self.hybrid_sparse_search_inner(
            dense_vector,
            sparse_query,
            k,
            strategy,
            Some(filter),
            DEFAULT_SPARSE_INDEX_NAME,
        )
    }

    /// Hybrid dense+sparse search on a named sparse index.
    ///
    /// Like [`hybrid_sparse_search`](Self::hybrid_sparse_search) but targets
    /// a specific named sparse index (e.g. for BGE-M3 multi-model embeddings).
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist or fusion fails.
    pub fn hybrid_sparse_search_named(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        k: usize,
        strategy: &FusionStrategy,
        index_name: &str,
    ) -> Result<Vec<SearchResult>> {
        self.hybrid_sparse_search_inner(dense_vector, sparse_query, k, strategy, None, index_name)
    }

    /// Hybrid dense+sparse search on a named sparse index with filtering.
    ///
    /// Combines [`hybrid_sparse_search_named`](Self::hybrid_sparse_search_named)
    /// with metadata filtering on the sparse branch.
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist or fusion fails.
    pub fn hybrid_sparse_search_named_with_filter(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        k: usize,
        strategy: &FusionStrategy,
        index_name: &str,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        self.hybrid_sparse_search_inner(
            dense_vector,
            sparse_query,
            k,
            strategy,
            Some(filter),
            index_name,
        )
    }

    /// Shared implementation for hybrid dense+sparse search with optional filter.
    fn hybrid_sparse_search_inner(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        k: usize,
        strategy: &FusionStrategy,
        filter: Option<&crate::filter::Filter>,
        index_name: &str,
    ) -> Result<Vec<SearchResult>> {
        let candidate_k = k.saturating_mul(2).max(k.saturating_add(10));

        let (dense_results, sparse_results) =
            self.execute_both_branches(dense_vector, sparse_query, index_name, candidate_k, filter);

        if dense_results.is_empty() && sparse_results.is_empty() {
            return Ok(Vec::new());
        }
        if dense_results.is_empty() {
            let scored: Vec<(u64, f32)> = sparse_results
                .iter()
                .map(|sd| (sd.doc_id, sd.score))
                .collect();
            return Ok(self.resolve_fused_results(&scored, k));
        }
        if sparse_results.is_empty() {
            return Ok(self.resolve_fused_results(&dense_results, k));
        }

        let sparse_tuples: Vec<(u64, f32)> = sparse_results
            .iter()
            .map(|sd| (sd.doc_id, sd.score))
            .collect();

        let fused = strategy
            .fuse(vec![dense_results, sparse_tuples])
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        Ok(self.resolve_fused_results(&fused, k))
    }
}
