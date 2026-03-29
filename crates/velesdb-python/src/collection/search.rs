//! Search methods for Collection (dense, sparse, hybrid, batch, multi-query).

use std::collections::HashMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use velesdb_core::FusionStrategy as CoreFusionStrategy;
use velesdb_core::SearchResult;

use crate::collection_helpers::{
    core_err, id_score_pairs_to_dicts, parse_filter, parse_optional_filter, parse_sparse_vector,
    search_result_to_dict, search_results_to_dicts,
};
use crate::utils::extract_vector;
use crate::FusionStrategy;

use super::Collection;

/// Default fusion strategy when none is specified by the caller.
const DEFAULT_FUSION: CoreFusionStrategy = CoreFusionStrategy::RRF { k: 60 };

/// A parsed batch search query ready for dispatch.
struct ParsedSearch {
    vector: Vec<f32>,
    top_k: usize,
    filter: Option<velesdb_core::Filter>,
}

#[pymethods]
impl Collection {
    /// Search for similar vectors (dense, sparse, or hybrid).
    ///
    /// Supports three modes depending on which arguments are provided:
    /// - Dense only: `search(vector, top_k=10)` (backward compatible)
    /// - Sparse only: `search(sparse_vector={0: 1.5, 3: 0.8}, top_k=10)`
    /// - Hybrid: `search(vector, sparse_vector={...}, top_k=10)` (fused with RRF k=60)
    ///
    /// Args:
    ///     vector: Dense query vector (list or numpy array). Optional if sparse_vector is given.
    ///     sparse_vector: Sparse query as dict[int, float] or scipy sparse. Optional if vector is given.
    ///     top_k: Number of results to return (default: 10).
    ///     filter: Optional metadata filter dict for pre-filtering results.
    ///     sparse_index_name: Optional name of the sparse index to query. When ``None``,
    ///         the default (unnamed) sparse index is used. Named sparse indexes are useful
    ///         for multi-model embeddings (e.g. BGE-M3 dense + sparse).
    ///
    /// Returns:
    ///     List of dicts with id, score, and payload.
    #[pyo3(signature = (vector=None, *, sparse_vector=None, top_k=10, filter=None, sparse_index_name=None))]
    fn search(
        &self,
        py: Python<'_>,
        vector: Option<PyObject>,
        sparse_vector: Option<PyObject>,
        top_k: usize,
        filter: Option<PyObject>,
        sparse_index_name: Option<String>,
    ) -> PyResult<Vec<PyObject>> {
        // Phase 1: Parse Python args (GIL held — required for PyObject access)
        let dense = vector.as_ref().map(|v| extract_vector(py, v)).transpose()?;
        let sparse = sparse_vector
            .as_ref()
            .map(|sv| parse_sparse_vector(py, sv))
            .transpose()?;
        let filter_obj = parse_optional_filter(py, filter)?;

        // Phase 2: Release GIL during Rust computation
        let results = py.allow_threads(|| {
            self.dispatch_search(
                dense,
                sparse,
                top_k,
                filter_obj.as_ref(),
                sparse_index_name.as_deref(),
            )
        })?;

        // Phase 3: Convert results (GIL held — required for PyObject creation)
        Ok(search_results_to_dicts(py, results))
    }

    /// Search for similar vectors with custom HNSW ef_search parameter.
    #[pyo3(signature = (vector, top_k = 10, ef_search = 128))]
    fn search_with_ef(
        &self,
        py: Python<'_>,
        vector: PyObject,
        top_k: usize,
        ef_search: usize,
    ) -> PyResult<Vec<PyObject>> {
        let query_vector = extract_vector(py, &vector)?;

        let results = py.allow_threads(|| {
            self.inner
                .search_with_ef(&query_vector, top_k, ef_search)
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results))
    }

    /// Search with a named quality mode (fast, balanced, accurate, perfect, autotune).
    ///
    /// AutoTune adapts ef_search automatically based on collection size and dimension.
    ///
    /// Args:
    ///     vector: Dense query vector (list or numpy array).
    ///     quality: Search quality mode string.
    ///     top_k: Number of results (default: 10).
    #[pyo3(signature = (vector, quality, top_k = 10))]
    fn search_with_quality(
        &self,
        py: Python<'_>,
        vector: PyObject,
        quality: &str,
        top_k: usize,
    ) -> PyResult<Vec<PyObject>> {
        let query_vector = extract_vector(py, &vector)?;
        let sq = parse_search_quality(quality)?;

        let results = py.allow_threads(|| {
            self.inner
                .search_with_quality(&query_vector, top_k, sq)
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results))
    }

    /// Search returning only IDs and scores.
    #[pyo3(signature = (vector, top_k = 10))]
    fn search_ids(
        &self,
        py: Python<'_>,
        vector: PyObject,
        top_k: usize,
    ) -> PyResult<Vec<PyObject>> {
        let query_vector = extract_vector(py, &vector)?;

        let results = py.allow_threads(|| {
            self.inner
                .search_ids(&query_vector, top_k)
                .map_err(core_err)
        })?;

        let tuples: Vec<(u64, f32)> = results.into_iter().map(Into::into).collect();
        Ok(id_score_pairs_to_dicts(py, tuples))
    }

    /// Search with metadata filtering.
    #[pyo3(signature = (vector, top_k = 10, filter = None))]
    fn search_with_filter(
        &self,
        py: Python<'_>,
        vector: PyObject,
        top_k: usize,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<PyObject>> {
        let query_vector = extract_vector(py, &vector)?;
        let filter_obj = filter
            .map(|f| parse_filter(py, &f))
            .transpose()?
            .ok_or_else(|| PyValueError::new_err("Filter is required for search_with_filter"))?;

        let results = py.allow_threads(|| {
            self.inner
                .search_with_filter(&query_vector, top_k, &filter_obj)
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results))
    }

    /// Full-text search using BM25 ranking.
    #[pyo3(signature = (query, top_k = 10, filter = None))]
    fn text_search(
        &self,
        py: Python<'_>,
        query: &str,
        top_k: usize,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<PyObject>> {
        let filter_obj = parse_optional_filter(py, filter)?;
        let query_owned = query.to_string();

        let results = py.allow_threads(|| {
            if let Some(f) = filter_obj {
                self.inner
                    .text_search_with_filter(&query_owned, top_k, &f)
                    .map_err(core_err)
            } else {
                self.inner
                    .text_search(&query_owned, top_k)
                    .map_err(core_err)
            }
        })?;

        Ok(search_results_to_dicts(py, results))
    }

    /// Hybrid search combining vector similarity and text search.
    #[pyo3(signature = (vector, query, top_k = 10, vector_weight = 0.5, filter = None))]
    fn hybrid_search(
        &self,
        py: Python<'_>,
        vector: PyObject,
        query: &str,
        top_k: usize,
        vector_weight: f32,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<PyObject>> {
        let query_vector = extract_vector(py, &vector)?;
        let filter_obj = parse_optional_filter(py, filter)?;
        let query_owned = query.to_string();

        let results = py.allow_threads(|| {
            if let Some(f) = filter_obj {
                self.inner.hybrid_search_with_filter(
                    &query_vector,
                    &query_owned,
                    top_k,
                    Some(vector_weight),
                    &f,
                )
            } else {
                self.inner.hybrid_search(
                    &query_vector,
                    &query_owned,
                    top_k,
                    Some(vector_weight),
                    None,
                )
            }
            .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results))
    }

    /// Batch search for multiple query vectors in parallel.
    ///
    /// Each search dict must contain a `"vector"` key and may optionally include
    /// `"top_k"` (or `"topK"`, default 10) and `"filter"`.
    ///
    /// Queries are partitioned by `top_k` so each group searches with the
    /// correct candidate count, avoiding wasted HNSW traversal when queries
    /// request different result sizes (issue #419).
    #[pyo3(signature = (searches))]
    fn batch_search(
        &self,
        py: Python<'_>,
        searches: Vec<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<Vec<PyObject>>> {
        let parsed = Self::parse_batch_searches(py, &searches)?;

        let results = py.allow_threads(|| self.dispatch_batch_by_top_k(&parsed))?;

        Ok(Self::convert_batch_results(py, results))
    }

    /// Multi-query search with result fusion.
    #[pyo3(signature = (vectors, top_k = 10, fusion = None, filter = None))]
    fn multi_query_search(
        &self,
        py: Python<'_>,
        vectors: Vec<PyObject>,
        top_k: usize,
        fusion: Option<FusionStrategy>,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<PyObject>> {
        let query_vectors: Vec<Vec<f32>> = vectors
            .iter()
            .map(|v| extract_vector(py, v))
            .collect::<PyResult<_>>()?;
        let fusion_strategy = fusion.map_or(DEFAULT_FUSION, |f| f.inner());
        let filter_obj = parse_optional_filter(py, filter)?;

        let results = py.allow_threads(|| {
            let query_refs: Vec<&[f32]> = query_vectors.iter().map(|v| v.as_slice()).collect();
            self.inner
                .multi_query_search(&query_refs, top_k, fusion_strategy, filter_obj.as_ref())
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results))
    }

    /// Multi-query search returning only IDs and fused scores.
    #[pyo3(signature = (vectors, top_k = 10, fusion = None))]
    fn multi_query_search_ids(
        &self,
        py: Python<'_>,
        vectors: Vec<PyObject>,
        top_k: usize,
        fusion: Option<FusionStrategy>,
    ) -> PyResult<Vec<PyObject>> {
        let query_vectors: Vec<Vec<f32>> = vectors
            .iter()
            .map(|v| extract_vector(py, v))
            .collect::<PyResult<_>>()?;
        let fusion_strategy = fusion.map_or(DEFAULT_FUSION, |f| f.inner());

        let results = py.allow_threads(|| {
            let query_refs: Vec<&[f32]> = query_vectors.iter().map(|v| v.as_slice()).collect();
            self.inner
                .multi_query_search_ids(&query_refs, top_k, fusion_strategy)
                .map_err(core_err)
        })?;

        let tuples: Vec<(u64, f32)> = results.into_iter().map(Into::into).collect();
        Ok(id_score_pairs_to_dicts(py, tuples))
    }
}

// ---------------------------------------------------------------------------
// Private helpers for batch_search (issue #419: per-query top_k).
// ---------------------------------------------------------------------------

impl Collection {
    /// Extracts vector, top_k, and filter from each search dict.
    fn parse_batch_searches(
        py: Python<'_>,
        searches: &[HashMap<String, PyObject>],
    ) -> PyResult<Vec<ParsedSearch>> {
        let mut parsed = Vec::with_capacity(searches.len());
        for search_dict in searches {
            let vector_obj = search_dict
                .get("vector")
                .ok_or_else(|| PyValueError::new_err("Search missing 'vector' field"))?;
            let vector = extract_vector(py, vector_obj)?;
            let top_k = search_dict
                .get("top_k")
                .or_else(|| search_dict.get("topK"))
                .map(|v| v.extract(py))
                .transpose()?
                .unwrap_or(10);
            let filter = search_dict
                .get("filter")
                .map(|f| parse_filter(py, f))
                .transpose()?;
            parsed.push(ParsedSearch {
                vector,
                top_k,
                filter,
            });
        }
        Ok(parsed)
    }

    /// Partitions queries by `top_k` and dispatches each group to the core
    /// batch search API, then reassembles results in original order.
    ///
    /// When all queries share the same `top_k` (common case), this collapses
    /// to a single core call with zero grouping overhead.
    fn dispatch_batch_by_top_k(&self, parsed: &[ParsedSearch]) -> PyResult<Vec<Vec<SearchResult>>> {
        if parsed.is_empty() {
            return Ok(Vec::new());
        }

        // Fast path: all queries share the same top_k (common case).
        let first_k = parsed[0].top_k;
        let all_same_k = parsed.iter().all(|p| p.top_k == first_k);
        if all_same_k {
            return self.dispatch_single_group(parsed, first_k);
        }

        self.dispatch_multi_group(parsed)
    }

    /// Dispatches all queries as a single batch (uniform top_k).
    fn dispatch_single_group(
        &self,
        parsed: &[ParsedSearch],
        k: usize,
    ) -> PyResult<Vec<Vec<SearchResult>>> {
        let query_refs: Vec<&[f32]> = parsed.iter().map(|p| p.vector.as_slice()).collect();
        let filters: Vec<Option<velesdb_core::Filter>> =
            parsed.iter().map(|p| p.filter.clone()).collect();
        self.inner
            .search_batch_with_filters(&query_refs, k, &filters)
            .map_err(core_err)
    }

    /// Groups queries by `top_k`, dispatches one batch per group, and
    /// reassembles results in the original input order.
    fn dispatch_multi_group(&self, parsed: &[ParsedSearch]) -> PyResult<Vec<Vec<SearchResult>>> {
        // Build groups: map top_k -> list of (original_index, query, filter).
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, p) in parsed.iter().enumerate() {
            groups.entry(p.top_k).or_default().push(i);
        }

        let mut output: Vec<Option<Vec<SearchResult>>> = vec![None; parsed.len()];

        for (k, indices) in &groups {
            let query_refs: Vec<&[f32]> = indices
                .iter()
                .map(|&i| parsed[i].vector.as_slice())
                .collect();
            let filters: Vec<Option<velesdb_core::Filter>> =
                indices.iter().map(|&i| parsed[i].filter.clone()).collect();

            let batch_results = self
                .inner
                .search_batch_with_filters(&query_refs, *k, &filters)
                .map_err(core_err)?;

            for (result, &orig_idx) in batch_results.into_iter().zip(indices) {
                output[orig_idx] = Some(result);
            }
        }

        // Invariant: every query index was assigned to exactly one group.
        debug_assert!(
            output.iter().all(Option::is_some),
            "batch dispatch left unassigned slots"
        );
        Ok(output.into_iter().map(|o| o.unwrap_or_default()).collect())
    }

    /// Converts core `SearchResult` vectors to Python dicts.
    fn convert_batch_results(
        py: Python<'_>,
        results: Vec<Vec<SearchResult>>,
    ) -> Vec<Vec<PyObject>> {
        results
            .iter()
            .map(|query_results| {
                query_results
                    .iter()
                    .map(|r| search_result_to_dict(py, r))
                    .collect()
            })
            .collect()
    }
}

/// Parse a Python quality mode string into [`SearchQuality`].
fn parse_search_quality(mode: &str) -> PyResult<velesdb_core::SearchQuality> {
    match mode.to_lowercase().as_str() {
        "fast" => Ok(velesdb_core::SearchQuality::Fast),
        "balanced" => Ok(velesdb_core::SearchQuality::Balanced),
        "accurate" => Ok(velesdb_core::SearchQuality::Accurate),
        "perfect" => Ok(velesdb_core::SearchQuality::Perfect),
        "autotune" | "auto_tune" | "auto" => Ok(velesdb_core::SearchQuality::AutoTune),
        other => Err(PyValueError::new_err(format!(
            "Unknown search quality: '{other}'. Valid: fast, balanced, accurate, perfect, autotune"
        ))),
    }
}
