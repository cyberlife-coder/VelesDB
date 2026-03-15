//! Search methods for Collection (dense, sparse, hybrid, batch, multi-query).

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;
use velesdb_core::FusionStrategy as CoreFusionStrategy;

use crate::collection_helpers::{
    id_score_pairs_to_dicts, parse_filter, parse_optional_filter, parse_sparse_vector,
    search_result_to_dict, search_results_to_dicts,
};
use crate::utils::extract_vector;
use crate::FusionStrategy;

use super::Collection;

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
        vector: Option<PyObject>,
        sparse_vector: Option<PyObject>,
        top_k: usize,
        filter: Option<PyObject>,
        sparse_index_name: Option<String>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let dense = vector.as_ref().map(|v| extract_vector(py, v)).transpose()?;
            let sparse = sparse_vector
                .as_ref()
                .map(|sv| parse_sparse_vector(py, sv))
                .transpose()?;
            let filter_obj = parse_optional_filter(py, filter)?;

            let results = self.dispatch_search(
                dense,
                sparse,
                top_k,
                filter_obj.as_ref(),
                sparse_index_name.as_deref(),
            )?;
            Ok(search_results_to_dicts(py, results))
        })
    }

    /// Search for similar vectors with custom HNSW ef_search parameter.
    #[pyo3(signature = (vector, top_k = 10, ef_search = 128))]
    fn search_with_ef(
        &self,
        vector: PyObject,
        top_k: usize,
        ef_search: usize,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let query_vector = extract_vector(py, &vector)?;
            let results = self
                .inner
                .search_with_ef(&query_vector, top_k, ef_search)
                .map_err(|e| PyRuntimeError::new_err(format!("Search with ef failed: {e}")))?;
            Ok(search_results_to_dicts(py, results))
        })
    }

    /// Search returning only IDs and scores.
    #[pyo3(signature = (vector, top_k = 10))]
    fn search_ids(
        &self,
        vector: PyObject,
        top_k: usize,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let query_vector = extract_vector(py, &vector)?;
            let results = self
                .inner
                .search_ids(&query_vector, top_k)
                .map_err(|e| PyRuntimeError::new_err(format!("Search IDs failed: {e}")))?;
            Ok(id_score_pairs_to_dicts(py, results))
        })
    }

    /// Search with metadata filtering.
    #[pyo3(signature = (vector, top_k = 10, filter = None))]
    fn search_with_filter(
        &self,
        vector: PyObject,
        top_k: usize,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let query_vector = extract_vector(py, &vector)?;
            let filter_obj = filter
                .map(|f| parse_filter(py, &f))
                .transpose()?
                .ok_or_else(|| {
                    PyValueError::new_err("Filter is required for search_with_filter")
                })?;
            let results = self
                .inner
                .search_with_filter(&query_vector, top_k, &filter_obj)
                .map_err(|e| PyRuntimeError::new_err(format!("Search with filter failed: {e}")))?;
            Ok(search_results_to_dicts(py, results))
        })
    }

    /// Full-text search using BM25 ranking.
    #[pyo3(signature = (query, top_k = 10, filter = None))]
    fn text_search(
        &self,
        query: &str,
        top_k: usize,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let filter_obj = parse_optional_filter(py, filter)?;
            let results = if let Some(f) = filter_obj {
                self.inner
                    .text_search_with_filter(query, top_k, &f)
                    .map_err(|e| PyRuntimeError::new_err(format!("Text search failed: {e}")))?
            } else {
                self.inner
                    .text_search(query, top_k)
                    .map_err(|e| PyRuntimeError::new_err(format!("Text search failed: {e}")))?
            };
            Ok(search_results_to_dicts(py, results))
        })
    }

    /// Hybrid search combining vector similarity and text search.
    #[pyo3(signature = (vector, query, top_k = 10, vector_weight = 0.5, filter = None))]
    fn hybrid_search(
        &self,
        vector: PyObject,
        query: &str,
        top_k: usize,
        vector_weight: f32,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let query_vector = extract_vector(py, &vector)?;
            let filter_obj = parse_optional_filter(py, filter)?;
            let results = if let Some(f) = filter_obj {
                self.inner.hybrid_search_with_filter(
                    &query_vector,
                    query,
                    top_k,
                    Some(vector_weight),
                    &f,
                )
            } else {
                self.inner
                    .hybrid_search(&query_vector, query, top_k, Some(vector_weight))
            }
            .map_err(|e| PyRuntimeError::new_err(format!("Hybrid search failed: {e}")))?;
            Ok(search_results_to_dicts(py, results))
        })
    }

    /// Batch search for multiple query vectors in parallel.
    #[pyo3(signature = (searches))]
    fn batch_search(
        &self,
        searches: Vec<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<Vec<HashMap<String, PyObject>>>> {
        Python::with_gil(|py| {
            let mut queries = Vec::with_capacity(searches.len());
            let mut filters = Vec::with_capacity(searches.len());
            let mut top_ks = Vec::with_capacity(searches.len());
            for search_dict in searches {
                let vector_obj = search_dict
                    .get("vector")
                    .ok_or_else(|| PyValueError::new_err("Search missing 'vector' field"))?;
                queries.push(extract_vector(py, vector_obj)?);
                top_ks.push(
                    search_dict
                        .get("top_k")
                        .or_else(|| search_dict.get("topK"))
                        .map(|v| v.extract(py))
                        .transpose()?
                        .unwrap_or(10),
                );
                filters.push(
                    search_dict
                        .get("filter")
                        .map(|f| parse_filter(py, f))
                        .transpose()?,
                );
            }
            let max_top_k = top_ks.iter().max().copied().unwrap_or(10);
            let query_refs: Vec<&[f32]> = queries.iter().map(|v| v.as_slice()).collect();
            let batch_results = self
                .inner
                .search_batch_with_filters(&query_refs, max_top_k, &filters)
                .map_err(|e| PyRuntimeError::new_err(format!("Batch search failed: {e}")))?;
            Ok(batch_results
                .into_iter()
                .zip(top_ks)
                .map(|(results, k)| {
                    results
                        .into_iter()
                        .take(k)
                        .map(|r| search_result_to_dict(py, &r))
                        .collect()
                })
                .collect())
        })
    }

    /// Multi-query search with result fusion.
    #[pyo3(signature = (vectors, top_k = 10, fusion = None, filter = None))]
    fn multi_query_search(
        &self,
        vectors: Vec<PyObject>,
        top_k: usize,
        fusion: Option<FusionStrategy>,
        filter: Option<PyObject>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let query_vectors: Vec<Vec<f32>> = vectors
                .iter()
                .map(|v| extract_vector(py, v))
                .collect::<PyResult<_>>()?;
            let fusion_strategy = fusion
                .map(|f| f.inner())
                .unwrap_or(CoreFusionStrategy::RRF { k: 60 });
            let filter_obj = parse_optional_filter(py, filter)?;
            let query_refs: Vec<&[f32]> = query_vectors.iter().map(|v| v.as_slice()).collect();
            let results = self
                .inner
                .multi_query_search(&query_refs, top_k, fusion_strategy, filter_obj.as_ref())
                .map_err(|e| PyRuntimeError::new_err(format!("Multi-query search failed: {e}")))?;
            Ok(search_results_to_dicts(py, results))
        })
    }

    /// Multi-query search returning only IDs and fused scores.
    #[pyo3(signature = (vectors, top_k = 10, fusion = None))]
    fn multi_query_search_ids(
        &self,
        vectors: Vec<PyObject>,
        top_k: usize,
        fusion: Option<FusionStrategy>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let query_vectors: Vec<Vec<f32>> = vectors
                .iter()
                .map(|v| extract_vector(py, v))
                .collect::<PyResult<_>>()?;
            let fusion_strategy = fusion
                .map(|f| f.inner())
                .unwrap_or(CoreFusionStrategy::RRF { k: 60 });
            let query_refs: Vec<&[f32]> = query_vectors.iter().map(|v| v.as_slice()).collect();
            let results = self
                .inner
                .multi_query_search_ids(&query_refs, top_k, fusion_strategy)
                .map_err(|e| {
                    PyRuntimeError::new_err(format!("Multi-query search IDs failed: {e}"))
                })?;
            Ok(id_score_pairs_to_dicts(py, results))
        })
    }
}
