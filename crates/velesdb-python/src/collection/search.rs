//! Search methods for Collection (dense, sparse, hybrid, batch, multi-query).

use std::collections::HashMap;

use pyo3::exceptions::{PyDeprecationWarning, PyValueError};
use pyo3::prelude::*;
use velesdb_core::FusionStrategy as CoreFusionStrategy;
use velesdb_core::SearchResult;

use crate::collection_helpers::{
    core_err, id_score_pairs_to_dicts, parse_filter, parse_optional_filter, parse_sparse_vector,
    search_result_to_dict, search_results_to_dicts,
};
use crate::utils::extract_vector;
use crate::FusionStrategy;

use super::search_options::SearchOptions;
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
    /// .. deprecated:: 1.15.0
    ///    Use :py:meth:`search_request` with a :py:class:`SearchOptions` object
    ///    instead.  This method will be removed in v2.0.0.
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
    // The too_many_arguments allow is preserved here because this legacy method
    // retains its original 6-kwarg signature for backward compatibility until
    // v2.0.  New callers should use search_request(SearchOptions) instead
    // (issue #717 v1.15 path).
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (vector=None, *, sparse_vector=None, top_k=10, filter=None, sparse_index_name=None, include_vectors=false))]
    fn search(
        &self,
        py: Python<'_>,
        vector: Option<Py<PyAny>>,
        sparse_vector: Option<Py<PyAny>>,
        top_k: usize,
        filter: Option<Py<PyAny>>,
        sparse_index_name: Option<String>,
        include_vectors: bool,
    ) -> PyResult<Vec<Py<PyAny>>> {
        PyErr::warn(
            py,
            &py.get_type::<PyDeprecationWarning>(),
            c"Collection.search() is deprecated since v1.15. \
              Use Collection.search_request(SearchOptions(...)) instead. \
              Will be removed in v2.0.",
            2,
        )?;

        let opts = SearchOptions::new(
            vector,
            sparse_vector,
            top_k,
            filter,
            sparse_index_name,
            include_vectors,
        );
        self.search_request(py, &opts)
    }

    /// Search for similar vectors using a :py:class:`SearchOptions` builder.
    ///
    /// This is the v1.15+ canonical search entry point.  It accepts all search
    /// parameters as a single ``SearchOptions`` object, eliminating the
    /// ``too_many_arguments`` lint suppression required by the legacy
    /// ``search()`` method (issue #717).
    ///
    /// Args:
    ///     opts: A :py:class:`SearchOptions` instance with all search parameters.
    ///
    /// Returns:
    ///     List of dicts with ``id``, ``score``, and ``payload`` keys.
    ///
    /// Example:
    ///     >>> opts = SearchOptions(vector=my_emb, top_k=20, filter={"lang": "en"})
    ///     >>> results = collection.search_request(opts)
    #[pyo3(signature = (opts))]
    fn search_request(&self, py: Python<'_>, opts: &SearchOptions) -> PyResult<Vec<Py<PyAny>>> {
        // Phase 1: Parse Python args (GIL held — required for Py<PyAny> access)
        let dense = opts
            .vector
            .as_ref()
            .map(|v| extract_vector(py, v))
            .transpose()?;
        let sparse = opts
            .sparse_vector
            .as_ref()
            .map(|sv| parse_sparse_vector(py, sv))
            .transpose()?;
        let filter_obj = parse_optional_filter(py, opts.filter.as_ref().map(|f| f.clone_ref(py)))?;

        let top_k = opts.top_k;
        let sparse_index_name = opts.sparse_index_name.clone();
        let include_vectors = opts.include_vectors;

        // Phase 2: Release GIL during Rust computation
        let results = py.detach(|| {
            self.dispatch_search(
                dense,
                sparse,
                top_k,
                filter_obj.as_ref(),
                sparse_index_name.as_deref(),
            )
        })?;

        // Phase 3: Convert results (GIL held — required for Py<PyAny> creation)
        Ok(search_results_to_dicts(py, results, include_vectors))
    }

    /// Search for similar vectors with custom HNSW ef_search parameter.
    #[pyo3(signature = (vector, top_k = 10, ef_search = 128))]
    fn search_with_ef(
        &self,
        py: Python<'_>,
        vector: Py<PyAny>,
        top_k: usize,
        ef_search: usize,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let query_vector = extract_vector(py, &vector)?;

        let results = py.detach(|| {
            self.inner
                .search_with_ef(&query_vector, top_k, ef_search)
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results, false))
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
        vector: Py<PyAny>,
        quality: &str,
        top_k: usize,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let query_vector = extract_vector(py, &vector)?;
        let sq = parse_search_quality(quality)?;

        let results = py.detach(|| {
            self.inner
                .search_with_quality(&query_vector, top_k, sq)
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results, false))
    }

    /// Search returning only IDs and scores.
    #[pyo3(signature = (vector, top_k = 10))]
    fn search_ids(
        &self,
        py: Python<'_>,
        vector: Py<PyAny>,
        top_k: usize,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let query_vector = extract_vector(py, &vector)?;

        let results = py.detach(|| {
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
        vector: Py<PyAny>,
        top_k: usize,
        filter: Option<Py<PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let query_vector = extract_vector(py, &vector)?;
        let filter_obj = filter
            .map(|f| parse_filter(py, &f))
            .transpose()?
            .ok_or_else(|| PyValueError::new_err("Filter is required for search_with_filter"))?;

        let results = py.detach(|| {
            self.inner
                .search_with_filter(&query_vector, top_k, &filter_obj)
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results, false))
    }

    /// Full-text search using BM25 ranking.
    #[pyo3(signature = (query, top_k = 10, filter = None))]
    fn text_search(
        &self,
        py: Python<'_>,
        query: &str,
        top_k: usize,
        filter: Option<Py<PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let filter_obj = parse_optional_filter(py, filter)?;
        let query_owned = query.to_string();

        let results = py.detach(|| {
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

        Ok(search_results_to_dicts(py, results, false))
    }

    /// Hybrid search combining vector similarity and text search.
    #[pyo3(signature = (vector, query, top_k = 10, vector_weight = 0.5, filter = None))]
    fn hybrid_search(
        &self,
        py: Python<'_>,
        vector: Py<PyAny>,
        query: &str,
        top_k: usize,
        vector_weight: f32,
        filter: Option<Py<PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let query_vector = extract_vector(py, &vector)?;
        let filter_obj = parse_optional_filter(py, filter)?;
        let query_owned = query.to_string();

        let results = py.detach(|| {
            if let Some(f) = filter_obj {
                self.inner.hybrid_search_with_filter(
                    &query_vector,
                    &query_owned,
                    top_k,
                    Some(vector_weight),
                    &f,
                )
            } else {
                self.inner
                    .hybrid_search(&query_vector, &query_owned, top_k, Some(vector_weight))
            }
            .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results, false))
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
        searches: Vec<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<Vec<Vec<Py<PyAny>>>> {
        let parsed = Self::parse_batch_searches(py, &searches)?;

        let results = py.detach(|| self.dispatch_batch_by_top_k(&parsed))?;

        Ok(Self::convert_batch_results(py, results))
    }

    /// Multi-query search with result fusion.
    #[pyo3(signature = (vectors, top_k = 10, fusion = None, filter = None))]
    fn multi_query_search(
        &self,
        py: Python<'_>,
        vectors: Vec<Py<PyAny>>,
        top_k: usize,
        fusion: Option<FusionStrategy>,
        filter: Option<Py<PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let query_vectors: Vec<Vec<f32>> = vectors
            .iter()
            .map(|v| extract_vector(py, v))
            .collect::<PyResult<_>>()?;
        let fusion_strategy = fusion.map_or(DEFAULT_FUSION, |f| f.inner());
        let filter_obj = parse_optional_filter(py, filter)?;

        let results = py.detach(|| {
            let query_refs: Vec<&[f32]> = query_vectors.iter().map(|v| v.as_slice()).collect();
            self.inner
                .multi_query_search(&query_refs, top_k, fusion_strategy, filter_obj.as_ref())
                .map_err(core_err)
        })?;

        Ok(search_results_to_dicts(py, results, false))
    }

    /// Parallel batch search for multiple query vectors.
    ///
    /// Each query is executed in parallel using rayon. All queries share the
    /// same ``top_k`` value. For per-query ``top_k`` control, use
    /// ``batch_search`` instead.
    ///
    /// Args:
    ///     vectors: List of query vectors (lists or numpy arrays).
    ///     top_k: Number of results per query (default: 10).
    ///
    /// Returns:
    ///     List of result lists, one per query vector.
    #[pyo3(signature = (vectors, top_k = 10))]
    fn search_batch_parallel(
        &self,
        py: Python<'_>,
        vectors: Vec<Py<PyAny>>,
        top_k: usize,
    ) -> PyResult<Vec<Vec<Py<PyAny>>>> {
        let query_vectors: Vec<Vec<f32>> = vectors
            .iter()
            .map(|v| extract_vector(py, v))
            .collect::<PyResult<_>>()?;

        let results = py.detach(|| {
            let query_refs: Vec<&[f32]> = query_vectors.iter().map(|v| v.as_slice()).collect();
            self.inner
                .search_batch_parallel(&query_refs, top_k)
                .map_err(core_err)
        })?;

        Ok(Self::convert_batch_results(py, results))
    }

    /// Multi-query search returning only IDs and fused scores.
    #[pyo3(signature = (vectors, top_k = 10, fusion = None))]
    fn multi_query_search_ids(
        &self,
        py: Python<'_>,
        vectors: Vec<Py<PyAny>>,
        top_k: usize,
        fusion: Option<FusionStrategy>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let query_vectors: Vec<Vec<f32>> = vectors
            .iter()
            .map(|v| extract_vector(py, v))
            .collect::<PyResult<_>>()?;
        let fusion_strategy = fusion.map_or(DEFAULT_FUSION, |f| f.inner());

        let results = py.detach(|| {
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
        searches: &[HashMap<String, Py<PyAny>>],
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
        output
            .into_iter()
            .enumerate()
            .map(|(i, slot)| {
                slot.ok_or_else(|| {
                    core_err(velesdb_core::error::Error::Query(format!(
                        "batch dispatch left slot {i} unassigned"
                    )))
                })
            })
            .collect::<PyResult<Vec<_>>>()
    }

    /// Converts core `SearchResult` vectors to Python dicts.
    fn convert_batch_results(
        py: Python<'_>,
        results: Vec<Vec<SearchResult>>,
    ) -> Vec<Vec<Py<PyAny>>> {
        results
            .iter()
            .map(|query_results| {
                query_results
                    .iter()
                    .map(|r| search_result_to_dict(py, r, false))
                    .collect()
            })
            .collect()
    }
}

/// Parse a Python quality mode string into [`SearchQuality`].
///
/// Supports named modes (`fast`, `balanced`, `accurate`, `perfect`, `autotune`)
/// plus advanced modes:
/// - `"custom:<ef>"` for a custom `ef_search` value
/// - `"adaptive:<min_ef>:<max_ef>"` for two-phase adaptive search
fn parse_search_quality(mode: &str) -> PyResult<velesdb_core::SearchQuality> {
    let lower = mode.to_lowercase();
    match lower.as_str() {
        "fast" => Ok(velesdb_core::SearchQuality::Fast),
        "balanced" => Ok(velesdb_core::SearchQuality::Balanced),
        "accurate" => Ok(velesdb_core::SearchQuality::Accurate),
        "perfect" => Ok(velesdb_core::SearchQuality::Perfect),
        "autotune" | "auto_tune" | "auto" => Ok(velesdb_core::SearchQuality::AutoTune),
        other => parse_advanced_quality(other),
    }
}

/// Parse advanced quality modes: `custom:<ef>` and `adaptive:<min_ef>:<max_ef>`.
fn parse_advanced_quality(mode: &str) -> PyResult<velesdb_core::SearchQuality> {
    if let Some(ef_str) = mode.strip_prefix("custom:") {
        let ef = ef_str.parse::<usize>().map_err(|_| {
            PyValueError::new_err(format!(
                "Invalid custom ef_search value: '{ef_str}'. Expected a positive integer, \
                 e.g. 'custom:256'"
            ))
        })?;
        return Ok(velesdb_core::SearchQuality::Custom(ef));
    }
    if let Some(params) = mode.strip_prefix("adaptive:") {
        return parse_adaptive_params(params);
    }
    Err(PyValueError::new_err(format!(
        "Unknown search quality: '{mode}'. Valid: fast, balanced, accurate, perfect, \
         autotune, custom:<ef>, adaptive:<min_ef>:<max_ef>"
    )))
}

/// Parse `<min_ef>:<max_ef>` for the adaptive quality mode.
fn parse_adaptive_params(params: &str) -> PyResult<velesdb_core::SearchQuality> {
    let parts: Vec<&str> = params.split(':').collect();
    if parts.len() != 2 {
        return Err(PyValueError::new_err(format!(
            "Invalid adaptive format: 'adaptive:{params}'. \
             Expected 'adaptive:<min_ef>:<max_ef>', e.g. 'adaptive:32:512'"
        )));
    }
    let min_ef = parts[0]
        .parse::<usize>()
        .map_err(|_| PyValueError::new_err(format!("Invalid adaptive min_ef: '{}'", parts[0])))?;
    let max_ef = parts[1]
        .parse::<usize>()
        .map_err(|_| PyValueError::new_err(format!("Invalid adaptive max_ef: '{}'", parts[1])))?;
    if min_ef > max_ef {
        return Err(PyValueError::new_err(format!(
            "Adaptive min_ef ({min_ef}) must be <= max_ef ({max_ef})"
        )));
    }
    Ok(velesdb_core::SearchQuality::Adaptive { min_ef, max_ef })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Initialize the Python interpreter once (idempotent, required by PyO3
    /// error constructors such as `PyValueError::new_err`).
    fn init_python() {
        pyo3::Python::initialize();
    }

    // ---- Named modes ----

    #[test]
    fn test_parse_named_modes() {
        init_python();
        assert!(matches!(
            parse_search_quality("fast").unwrap(),
            velesdb_core::SearchQuality::Fast
        ));
        assert!(matches!(
            parse_search_quality("balanced").unwrap(),
            velesdb_core::SearchQuality::Balanced
        ));
        assert!(matches!(
            parse_search_quality("accurate").unwrap(),
            velesdb_core::SearchQuality::Accurate
        ));
        assert!(matches!(
            parse_search_quality("perfect").unwrap(),
            velesdb_core::SearchQuality::Perfect
        ));
        assert!(matches!(
            parse_search_quality("autotune").unwrap(),
            velesdb_core::SearchQuality::AutoTune
        ));
        assert!(matches!(
            parse_search_quality("auto").unwrap(),
            velesdb_core::SearchQuality::AutoTune
        ));
    }

    #[test]
    fn test_parse_named_modes_case_insensitive() {
        init_python();
        assert!(matches!(
            parse_search_quality("FAST").unwrap(),
            velesdb_core::SearchQuality::Fast
        ));
        assert!(matches!(
            parse_search_quality("Balanced").unwrap(),
            velesdb_core::SearchQuality::Balanced
        ));
    }

    // ---- Custom mode ----

    #[test]
    fn test_parse_custom_valid() {
        init_python();
        let q = parse_search_quality("custom:256").unwrap();
        assert!(matches!(q, velesdb_core::SearchQuality::Custom(256)));
    }

    #[test]
    fn test_parse_custom_case_insensitive() {
        init_python();
        let q = parse_search_quality("Custom:128").unwrap();
        assert!(matches!(q, velesdb_core::SearchQuality::Custom(128)));
    }

    #[test]
    fn test_parse_custom_invalid_value() {
        init_python();
        let err = parse_search_quality("custom:abc").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid custom ef_search"), "got: {msg}");
    }

    #[test]
    fn test_parse_custom_empty_value() {
        init_python();
        let err = parse_search_quality("custom:").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid custom ef_search"), "got: {msg}");
    }

    // ---- Adaptive mode ----

    #[test]
    fn test_parse_adaptive_valid() {
        init_python();
        let q = parse_search_quality("adaptive:32:512").unwrap();
        assert!(matches!(
            q,
            velesdb_core::SearchQuality::Adaptive {
                min_ef: 32,
                max_ef: 512
            }
        ));
    }

    #[test]
    fn test_parse_adaptive_equal_bounds() {
        init_python();
        let q = parse_search_quality("adaptive:100:100").unwrap();
        assert!(matches!(
            q,
            velesdb_core::SearchQuality::Adaptive {
                min_ef: 100,
                max_ef: 100
            }
        ));
    }

    #[test]
    fn test_parse_adaptive_case_insensitive() {
        init_python();
        let q = parse_search_quality("Adaptive:16:256").unwrap();
        assert!(matches!(
            q,
            velesdb_core::SearchQuality::Adaptive {
                min_ef: 16,
                max_ef: 256
            }
        ));
    }

    #[test]
    fn test_parse_adaptive_inverted_range() {
        init_python();
        let err = parse_search_quality("adaptive:512:32").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be <= max_ef"), "got: {msg}");
    }

    #[test]
    fn test_parse_adaptive_missing_max() {
        init_python();
        let err = parse_search_quality("adaptive:32").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid adaptive format"), "got: {msg}");
    }

    #[test]
    fn test_parse_adaptive_non_numeric() {
        init_python();
        let err = parse_search_quality("adaptive:a:b").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid adaptive min_ef"), "got: {msg}");
    }

    // ---- Unknown mode ----

    #[test]
    fn test_parse_unknown_mode() {
        init_python();
        let err = parse_search_quality("nonexistent").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Unknown search quality"), "got: {msg}");
        assert!(
            msg.contains("custom:<ef>"),
            "error should mention custom syntax: {msg}"
        );
        assert!(
            msg.contains("adaptive:<min_ef>:<max_ef>"),
            "error should mention adaptive syntax: {msg}"
        );
    }
}
