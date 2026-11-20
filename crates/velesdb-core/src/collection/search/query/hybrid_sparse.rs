//! Hybrid dense+sparse search execution with RRF/RSF fusion.
//!
//! Extracts `SparseVectorSearch` from the AST condition tree, executes
//! sparse (optionally filtered) and dense searches in parallel, and fuses
//! results using the requested fusion strategy.

use crate::collection::search::query::row_universe::RowScope;
use crate::collection::search::ranked::Ranked;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::fusion::FusionStrategy;
use crate::index::sparse::{SparseVector, DEFAULT_SPARSE_INDEX_NAME};
use crate::point::SearchResult;
use crate::sparse_index::search::sparse_scores;
use crate::velesql::{Condition, SparseVectorSearch};

/// The two ranked branches of a hybrid search, before fusion: the dense hits
/// as `(id, score)` pairs, the sparse hits as scored documents.
type HybridBranches = (Vec<(u64, f32)>, Vec<crate::index::sparse::ScoredDoc>);

impl Collection {
    // ------------------------------------------------------------------
    // Condition extraction
    // ------------------------------------------------------------------

    /// Recursively walks the condition tree to find a `SparseVectorSearch` node.
    ///
    /// A query has at most one, never under `NOT`:
    /// `validate_similarity_query_structure` rejects a second `SPARSE_NEAR`,
    /// and one under `NOT`, before this runs, so the left-most node it
    /// returns is the only one.
    pub(crate) fn extract_sparse_vector_search(
        condition: &Condition,
    ) -> Option<&SparseVectorSearch> {
        match condition {
            Condition::SparseVectorSearch(svs) => Some(svs),
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::extract_sparse_vector_search(left)
                    .or_else(|| Self::extract_sparse_vector_search(right))
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::extract_sparse_vector_search(inner)
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------------
    // Sparse-only execution
    // ------------------------------------------------------------------

    /// Execute a sparse-only search, optionally filtered by payload conditions.
    pub(crate) fn execute_sparse_search(
        &self,
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.execute_sparse_search_in_anchors(svs, params, filter_condition, limit, None)
    }

    /// Execute a sparse-only search, optionally filtered by payload
    /// conditions and/or restricted to a GraphFirst anchor-id set.
    ///
    /// Both are part of the walk's admission, so the fetch is exact at
    /// `limit` among the rows they admit — no over-fetch window
    /// ([`Self::sparse_hits`]).
    pub(crate) fn execute_sparse_search_in_anchors(
        &self,
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
        anchors: Option<&std::collections::HashSet<u64>>,
    ) -> Result<Vec<SearchResult>> {
        let query_vec = svs.vector.resolve(params)?;
        let index_name = svs
            .index_name
            .as_deref()
            .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);

        // Build payload filter if there are non-vector metadata conditions.
        let metadata_filter = filter_condition
            .and_then(Self::extract_metadata_filter)
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond)));

        self.sparse_hits(
            index_name,
            &query_vec,
            limit,
            metadata_filter.as_ref(),
            anchors,
        )
    }

    /// The best `k` hits of `query` on the sparse index `index_name`,
    /// hydrated: the collection's rows ([`RowScope::All`]) — given
    /// `anchors`, among those ids, and given a payload `filter`, those whose
    /// payload it matches — all decided by the walk's admission before the
    /// top-`k` cut ([`Collection::ranked_hits`]), so a posting left for an
    /// id that is no row (a vector collection's id without a vector, from an
    /// earlier version) cannot crowd a row out, and a matching row is found
    /// however far below the best it ranks.
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist.
    ///
    /// LOCK ORDER: config(1), released; sparse_indexes(9) alone across the
    /// scoring, released; secondary_indexes(6) alone for the prefilter,
    /// released; then vector_storage(2) → payload_storage(3), both read, per
    /// batch of the walk ([`Collection::ranked_hits`]).
    pub(crate) fn sparse_hits(
        &self,
        index_name: &str,
        query: &SparseVector,
        k: usize,
        filter: Option<&crate::filter::Filter>,
        anchors: Option<&std::collections::HashSet<u64>>,
    ) -> Result<Vec<SearchResult>> {
        let rows = RowScope::All.universe(self.row_universe());
        let ranked = self.sparse_ranked(index_name, query, filter)?;
        Ok(self.ranked_hits(
            ranked,
            k,
            rows,
            |id| anchors.is_none_or(|a| a.contains(&id)),
            |id, payload| filter.is_none_or(|f| f.matches_point(id, payload)),
        ))
    }

    /// The best `k` hits of `query` on the sparse index `index_name` among
    /// the collection's rows ([`RowScope::All`]) whose payload `filter`, when
    /// given, matches, as scored documents: the sparse branch of a hybrid
    /// search, which fusion ranks again ([`Collection::ranked_rows`]).
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist.
    ///
    /// LOCK ORDER: config(1), released; sparse_indexes(9) alone across the
    /// scoring, released; secondary_indexes(6) alone for the prefilter,
    /// released; then vector_storage(2) → payload_storage(3), both read, per
    /// batch of the walk ([`Collection::ranked_rows`]).
    pub(crate) fn sparse_rows(
        &self,
        index_name: &str,
        query: &SparseVector,
        k: usize,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<crate::index::sparse::ScoredDoc>> {
        let rows = RowScope::All.universe(self.row_universe());
        let ranked = self.sparse_ranked(index_name, query, filter)?;
        Ok(self
            .ranked_rows(ranked, k, rows, |_| true, filter)
            .into_iter()
            .map(|(doc_id, score)| crate::index::sparse::ScoredDoc { score, doc_id })
            .collect())
    }

    /// Every hit of `query` on the sparse index `index_name`, scored under
    /// `sparse_indexes`' read guard alone, but those `filter`'s prefilter
    /// bitmap rules out ([`Collection::ranked_within_prefilter`]).
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist.
    fn sparse_ranked(
        &self,
        index_name: &str,
        query: &SparseVector,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Ranked> {
        let indexes = self.query.sparse_indexes.read();
        let index = indexes
            .get(index_name)
            .ok_or_else(|| Error::sparse_index_not_found(index_name))?;
        let mut scored = Vec::new();
        sparse_scores(index, query, |doc_id, score| scored.push((doc_id, score)));
        drop(indexes);
        Ok(self.ranked_within_prefilter(scored, filter))
    }

    // ------------------------------------------------------------------
    // Hybrid dense+sparse execution
    // ------------------------------------------------------------------

    /// Execute hybrid search with an explicit fusion strategy.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_hybrid_search_with_strategy(
        &self,
        dense_vector: &[f32],
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
        strategy: &FusionStrategy,
    ) -> Result<Vec<SearchResult>> {
        let sparse_query = svs.vector.resolve(params)?;
        let index_name = svs
            .index_name
            .as_deref()
            .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);
        let candidate_k = crate::fusion::dense_sparse_candidate_k(limit);

        // Pre-build metadata filter for the sparse branch.
        let metadata_filter = filter_condition
            .and_then(Self::extract_metadata_filter)
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond)));

        // Execute both branches.
        let (dense_results, sparse_results) = self.execute_both_branches(
            dense_vector,
            &sparse_query,
            index_name,
            candidate_k,
            metadata_filter.as_ref(),
        )?;

        self.fuse_dense_sparse(dense_results, &sparse_results, limit, strategy)
    }

    /// Execute the dense branch, then the sparse branch.
    ///
    /// Either branch's failure fails the hybrid search — a dense query of the
    /// wrong dimension, an unknown sparse index — instead of leaving that
    /// branch empty for [`fuse_dense_sparse`](Self::fuse_dense_sparse) to
    /// degrade over. A branch that ran and matched nothing is still an empty,
    /// successful branch. When both fail, the dense error is reported.
    ///
    /// # Lock ordering
    ///
    /// The sparse branch scores under `sparse_indexes` (9) alone, then
    /// admits and filters its hits under `vector_storage` (2) and
    /// `payload_storage` (3) ([`Self::sparse_rows`]).
    ///
    /// The branches run one after the other, on this thread. Both take
    /// collection locks — the dense one `vector_storage` (2) for the points
    /// not yet indexed, the sparse one `sparse_indexes` (9), then
    /// `vector_storage` (2) and `payload_storage` (3) — and no job on a
    /// rayon pool may take a collection lock (`docs/CONCURRENCY_MODEL.md`,
    /// "Collection locks and rayon").
    ///
    /// # Errors
    ///
    /// Returns the dense branch's error (e.g. a dimension mismatch), else the
    /// sparse branch's (e.g. the named sparse index does not exist).
    pub(crate) fn execute_both_branches(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        index_name: &str,
        candidate_k: usize,
        metadata_filter: Option<&crate::filter::Filter>,
    ) -> Result<HybridBranches> {
        let dense = self.dense_branch(dense_vector, candidate_k, metadata_filter);
        let sparse = self.sparse_branch(sparse_query, index_name, candidate_k, metadata_filter);
        Ok((dense?, sparse?))
    }

    /// The dense (HNSW) branch of a hybrid search, as `(id, score)` pairs.
    ///
    /// The metadata filter bounds BOTH branches. The dense branch previously
    /// ignored it entirely (only sparse filtered), so dense-only candidates the
    /// filter excludes leaked into fusion. `search_ids` acquires
    /// `vector_storage(2)` only — no conflict with the sparse branch's locks.
    fn dense_branch(
        &self,
        dense_vector: &[f32],
        candidate_k: usize,
        metadata_filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<(u64, f32)>> {
        Ok(match metadata_filter {
            Some(filter) => self
                .search_with_filter(dense_vector, candidate_k, filter)?
                .into_iter()
                .map(|r| (r.point.id, r.score))
                .collect(),
            None => self
                .search_ids(dense_vector, candidate_k)?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    /// The sparse branch of a hybrid search on the named sparse index: its
    /// best `candidate_k` rows ([`Self::sparse_rows`]).
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist, as the
    /// sparse-only search does.
    fn sparse_branch(
        &self,
        sparse_query: &SparseVector,
        index_name: &str,
        candidate_k: usize,
        metadata_filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<crate::index::sparse::ScoredDoc>> {
        self.sparse_rows(index_name, sparse_query, candidate_k, metadata_filter)
    }

    /// Fuse dense and sparse branch results into ranked `SearchResult`s.
    ///
    /// A branch that came back empty is still fused, as an empty list: the
    /// scores returned always have the direction
    /// [`FusionStrategy::fused_direction`] states for a dense and a sparse
    /// branch, so a caller reading them as fused scores never meets the dense
    /// branch's raw distances instead. Shared by the VelesQL hybrid path and
    /// the raw `SparseVector` SDK entry points.
    ///
    /// # Errors
    ///
    /// Returns an error if the fusion strategy itself fails.
    pub(crate) fn fuse_dense_sparse(
        &self,
        dense_results: Vec<(u64, f32)>,
        sparse_results: &[crate::index::sparse::ScoredDoc],
        limit: usize,
        strategy: &FusionStrategy,
    ) -> Result<Vec<SearchResult>> {
        if dense_results.is_empty() && sparse_results.is_empty() {
            return Ok(Vec::new());
        }

        let sparse_tuples: Vec<(u64, f32)> = sparse_results
            .iter()
            .map(|sd| (sd.doc_id, sd.score))
            .collect();

        // Dense scores carry the collection metric's polarity (distances for
        // Euclidean/Hamming); sparse scores are always higher-is-better
        // relevance. Score-level strategies must align them (#2102).
        let directions = [
            self.storage.config.read().metric.score_direction(),
            crate::fusion::ScoreDirection::HigherIsBetter,
        ];
        let fused =
            strategy.fuse_with_directions(vec![dense_results, sparse_tuples], &directions)?;

        Ok(self.resolve_fused_results(&fused, limit))
    }

    // ------------------------------------------------------------------
    // Result resolution helpers
    // ------------------------------------------------------------------

    /// Resolve fused `(id, score)` tuples to `SearchResult`: the first
    /// `limit` that are rows of the collection ([`RowScope::All`]) and live
    /// ([`Collection::first_rows`]).
    pub(crate) fn resolve_fused_results(
        &self,
        fused: &[(u64, f32)],
        limit: usize,
    ) -> Vec<SearchResult> {
        self.first_rows(
            fused.iter().copied(),
            limit,
            RowScope::All.universe(self.row_universe()),
        )
    }
}
