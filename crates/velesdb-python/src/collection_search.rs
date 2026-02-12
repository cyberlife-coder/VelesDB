//! Search methods for Collection (extracted from collection.rs).
//!
//! Contains: search, search_with_filter, text_search, hybrid_search,
//! batch_search, multi_query_search, multi_query_search_ids,
//! search_with_ef, search_ids.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::collection::Collection;
use crate::collection_helpers::{
    id_score_pairs_to_dicts, parse_filter, parse_optional_filter, search_result_to_dict,
    search_results_to_dicts,
};
use crate::utils::extract_vector;
use crate::FusionStrategy;
use velesdb_core::FusionStrategy as CoreFusionStrategy;

#[pymethods]
impl Collection {
    /// Search for similar vectors.
    #[pyo3(signature = (vector, top_k = 10))]
    fn search(&self, vector: PyObject, top_k: usize) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let query_vector = extract_vector(py, &vector)?;
            let results = self
                .inner
                .search(&query_vector, top_k)
                .map_err(|e| PyRuntimeError::new_err(format!("Search failed: {}", e)))?;

            Ok(search_results_to_dicts(py, results))
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
                self.inner.text_search_with_filter(query, top_k, &f)
            } else {
                self.inner.text_search(query, top_k)
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

    // ========================================================================
    // Advanced Search (Phase 4.3 Plan 02)
    // ========================================================================

    /// Search with custom ef_search parameter for HNSW tuning.
    ///
    /// Higher ef_search increases recall but is slower.
    /// Default ef_search is 128. Use 200-500 for higher recall.
    ///
    /// Args:
    ///     vector: Query vector
    ///     top_k: Number of results (default: 10)
    ///     ef_search: HNSW ef_search parameter (default: 100)
    ///
    /// Returns:
    ///     List of dicts with keys: id, score, payload
    ///
    /// Example:
    ///     >>> results = collection.search_with_ef([0.1, 0.2, ...], top_k=10, ef_search=200)
    #[pyo3(signature = (vector, top_k = 10, ef_search = 100))]
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

    /// Lightweight search returning only IDs and scores (no payload).
    ///
    /// Faster than search() when you only need IDs (~3-5x speedup).
    ///
    /// Args:
    ///     vector: Query vector
    ///     top_k: Number of results (default: 10)
    ///
    /// Returns:
    ///     List of tuples (id, score)
    ///
    /// Example:
    ///     >>> ids_scores = collection.search_ids([0.1, 0.2, ...], top_k=10)
    ///     >>> for id, score in ids_scores:
    ///     ...     print(f"ID: {id}, Score: {score:.3f}")
    #[pyo3(signature = (vector, top_k = 10))]
    fn search_ids(&self, vector: PyObject, top_k: usize) -> PyResult<Vec<(u64, f32)>> {
        Python::with_gil(|py| {
            let query_vector = extract_vector(py, &vector)?;
            self.inner
                .search_ids(&query_vector, top_k)
                .map_err(|e| PyRuntimeError::new_err(format!("Search IDs failed: {e}")))
        })
    }
}
