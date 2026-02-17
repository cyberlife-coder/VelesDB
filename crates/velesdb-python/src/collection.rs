//! Collection module for VelesDB Python bindings.
//!
//! This module contains the `Collection` struct and all its PyO3 methods
//! for vector storage and similarity search operations.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::collection_helpers::{
    id_score_pairs_to_dicts, parse_filter, parse_optional_filter, point_to_dict,
    search_result_to_dict, search_results_to_dicts, search_results_to_multimodel_dicts,
};
use crate::utils::{extract_vector, json_to_python, python_to_json, to_pyobject};
use crate::FusionStrategy;
use velesdb_core::{FusionStrategy as CoreFusionStrategy, Point};

/// A vector collection in VelesDB.
///
/// Collections store vectors with optional metadata (payload) and support
/// efficient similarity search.
#[pyclass]
pub struct Collection {
    pub(crate) inner: Arc<velesdb_core::Collection>,
    pub(crate) name: String,
}

impl Collection {
    /// Create a new Collection wrapper.
    pub fn new(inner: Arc<velesdb_core::Collection>, name: String) -> Self {
        Self { inner, name }
    }
}

#[pymethods]
impl Collection {
    /// Get the collection name.
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// Get collection configuration info.
    ///
    /// Returns:
    ///     Dict with name, dimension, metric, storage_mode, point_count, and metadata_only
    fn info(&self) -> PyResult<HashMap<String, PyObject>> {
        Python::with_gil(|py| {
            let config = self.inner.config();
            let mut info = HashMap::new();
            info.insert("name".to_string(), to_pyobject(py, config.name.as_str()));
            info.insert("dimension".to_string(), to_pyobject(py, config.dimension));
            info.insert(
                "metric".to_string(),
                to_pyobject(py, format!("{:?}", config.metric).to_lowercase()),
            );
            info.insert(
                "storage_mode".to_string(),
                to_pyobject(py, format!("{:?}", config.storage_mode).to_lowercase()),
            );
            info.insert(
                "point_count".to_string(),
                to_pyobject(py, config.point_count),
            );
            info.insert(
                "metadata_only".to_string(),
                to_pyobject(py, config.metadata_only),
            );
            Ok(info)
        })
    }

    /// Check if this is a metadata-only collection.
    fn is_metadata_only(&self) -> bool {
        self.inner.is_metadata_only()
    }

    /// Insert or update vectors in the collection.
    #[pyo3(signature = (points))]
    fn upsert(&self, points: Vec<HashMap<String, PyObject>>) -> PyResult<usize> {
        Python::with_gil(|py| {
            let mut core_points = Vec::with_capacity(points.len());

            for point_dict in points {
                let id: u64 = point_dict
                    .get("id")
                    .ok_or_else(|| PyValueError::new_err("Point missing 'id' field"))?
                    .extract(py)?;

                let vector_obj = point_dict
                    .get("vector")
                    .ok_or_else(|| PyValueError::new_err("Point missing 'vector' field"))?;
                let vector = extract_vector(py, vector_obj)?;

                let payload: Option<serde_json::Value> = match point_dict.get("payload") {
                    Some(p) => {
                        let payload_str: String = p
                            .call_method0(py, "__str__")
                            .and_then(|s| s.extract(py))
                            .ok()
                            .unwrap_or_default();

                        if let Ok(json_val) = serde_json::from_str(&payload_str) {
                            Some(json_val)
                        } else {
                            let dict: HashMap<String, PyObject> =
                                p.extract(py).ok().unwrap_or_default();
                            let json_map: serde_json::Map<String, serde_json::Value> = dict
                                .into_iter()
                                .filter_map(|(k, v)| python_to_json(py, &v).map(|jv| (k, jv)))
                                .collect();
                            Some(serde_json::Value::Object(json_map))
                        }
                    }
                    None => None,
                };

                core_points.push(Point::new(id, vector, payload));
            }

            let count = core_points.len();
            self.inner
                .upsert(core_points)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to upsert: {e}")))?;

            Ok(count)
        })
    }

    /// Insert or update metadata-only points (no vectors).
    #[pyo3(signature = (points))]
    fn upsert_metadata(&self, points: Vec<HashMap<String, PyObject>>) -> PyResult<usize> {
        Python::with_gil(|py| {
            let mut core_points = Vec::with_capacity(points.len());

            for point_dict in points {
                let id: u64 = point_dict
                    .get("id")
                    .ok_or_else(|| PyValueError::new_err("Point missing 'id' field"))?
                    .extract(py)?;

                let payload: serde_json::Value = match point_dict.get("payload") {
                    Some(p) => {
                        let dict: HashMap<String, PyObject> =
                            p.extract(py).ok().unwrap_or_default();
                        let json_map: serde_json::Map<String, serde_json::Value> = dict
                            .into_iter()
                            .filter_map(|(k, v)| python_to_json(py, &v).map(|jv| (k, jv)))
                            .collect();
                        serde_json::Value::Object(json_map)
                    }
                    None => {
                        return Err(PyValueError::new_err(
                            "Metadata-only point must have 'payload' field",
                        ))
                    }
                };

                core_points.push(Point::metadata_only(id, payload));
            }

            let count = core_points.len();
            self.inner
                .upsert_metadata(core_points)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to upsert_metadata: {e}")))?;

            Ok(count)
        })
    }

    /// Bulk insert optimized for high-throughput import.
    #[pyo3(signature = (points))]
    fn upsert_bulk(&self, points: Vec<HashMap<String, PyObject>>) -> PyResult<usize> {
        Python::with_gil(|py| {
            let mut core_points = Vec::with_capacity(points.len());

            for point_dict in points {
                let id: u64 = point_dict
                    .get("id")
                    .ok_or_else(|| PyValueError::new_err("Point missing 'id' field"))?
                    .extract(py)?;

                let vector_obj = point_dict
                    .get("vector")
                    .ok_or_else(|| PyValueError::new_err("Point missing 'vector' field"))?;
                let vector = extract_vector(py, vector_obj)?;

                let payload: Option<serde_json::Value> = match point_dict.get("payload") {
                    Some(p) => {
                        let dict: HashMap<String, PyObject> =
                            p.extract(py).ok().unwrap_or_default();
                        let json_map: serde_json::Map<String, serde_json::Value> = dict
                            .into_iter()
                            .filter_map(|(k, v)| python_to_json(py, &v).map(|jv| (k, jv)))
                            .collect();
                        Some(serde_json::Value::Object(json_map))
                    }
                    None => None,
                };

                core_points.push(Point::new(id, vector, payload));
            }

            self.inner
                .upsert_bulk(&core_points)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to upsert_bulk: {}", e)))
        })
    }

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

    /// Get points by their IDs.
    #[pyo3(signature = (ids))]
    fn get(&self, ids: Vec<u64>) -> PyResult<Vec<Option<HashMap<String, PyObject>>>> {
        Python::with_gil(|py| {
            let points = self.inner.get(&ids);
            let py_points = points
                .into_iter()
                .map(|opt_point| opt_point.map(|p| point_to_dict(py, &p)))
                .collect();
            Ok(py_points)
        })
    }

    /// Delete points by their IDs.
    #[pyo3(signature = (ids))]
    fn delete(&self, ids: Vec<u64>) -> PyResult<()> {
        self.inner
            .delete(&ids)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete: {}", e)))
    }

    /// Check if the collection is empty.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Flush all pending changes to disk.
    fn flush(&self) -> PyResult<()> {
        self.inner
            .flush()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to flush: {}", e)))
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

    /// Execute a VelesQL query (EPIC-031 US-008).
    ///
    /// Executes SELECT-style VelesQL queries with vector similarity search.
    ///
    /// Args:
    ///     query_str: VelesQL SELECT query string
    ///     params: Query parameters (vectors as lists/numpy arrays, scalars)
    ///
    /// Returns:
    ///     List of query results with node_id, vector_score, graph_score,
    ///     fused_score, bindings (payload), and column_data
    ///
    /// Example:
    ///     >>> results = collection.query(
    ///     ...     "SELECT * FROM docs WHERE vector NEAR $q LIMIT 20",
    ///     ...     params={"q": query_embedding}
    ///     ... )
    ///     >>> for r in results:
    ///     ...     print(f"Node: {r['node_id']}, Score: {r['fused_score']:.3f}")
    #[pyo3(signature = (query_str, params=None))]
    fn query(
        &self,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;

            let rust_params: std::collections::HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = self
                .inner
                .execute_query(&parsed, &rust_params)
                .map_err(|e| PyRuntimeError::new_err(format!("Query failed: {e}")))?;

            Ok(search_results_to_multimodel_dicts(py, results))
        })
    }

    /// Execute a MATCH graph traversal query.
    ///
    /// Args:
    ///     query_str: VelesQL MATCH query string
    ///     params: Query parameters (default: empty dict)
    ///     vector: Optional query vector for similarity scoring
    ///     threshold: Similarity threshold (default: 0.0)
    ///
    /// Returns:
    ///     List of dicts with keys: node_id, depth, path, bindings, score, projected
    #[pyo3(signature = (query_str, params = None, vector = None, threshold = 0.0))]
    fn match_query(
        &self,
        query_str: &str,
        params: Option<HashMap<String, PyObject>>,
        vector: Option<PyObject>,
        threshold: f32,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;
            let match_clause = parsed
                .match_clause
                .as_ref()
                .ok_or_else(|| PyValueError::new_err("Query is not a MATCH query"))?;

            let rust_params: std::collections::HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = if let Some(vector_obj) = vector {
                let query_vector = extract_vector(py, &vector_obj)?;
                self.inner
                    .execute_match_with_similarity(
                        match_clause,
                        &query_vector,
                        threshold,
                        &rust_params,
                    )
                    .map_err(|e| PyRuntimeError::new_err(format!("MATCH query failed: {e}")))?
            } else {
                self.inner
                    .execute_match(match_clause, &rust_params)
                    .map_err(|e| PyRuntimeError::new_err(format!("MATCH query failed: {e}")))?
            };

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut dict = HashMap::new();
                    dict.insert("node_id".to_string(), to_pyobject(py, r.node_id));
                    dict.insert("depth".to_string(), to_pyobject(py, r.depth));
                    dict.insert("path".to_string(), to_pyobject(py, r.path));
                    dict.insert("bindings".to_string(), to_pyobject(py, r.bindings));
                    dict.insert("score".to_string(), to_pyobject(py, r.score));
                    let projected: HashMap<String, PyObject> = r
                        .projected
                        .into_iter()
                        .map(|(k, v)| (k, json_to_python(py, &v)))
                        .collect();
                    dict.insert("projected".to_string(), to_pyobject(py, projected));
                    dict
                })
                .collect();
            Ok(py_results)
        })
    }

    /// Return query execution plan (EXPLAIN).
    #[pyo3(signature = (query_str))]
    fn explain(&self, query_str: &str) -> PyResult<HashMap<String, PyObject>> {
        Python::with_gil(|py| {
            let parsed = velesdb_core::velesql::Parser::parse(query_str).map_err(|e| {
                PyValueError::new_err(format!("VelesQL parse error: {}", e.message))
            })?;

            let plan = if let Some(match_clause) = parsed.match_clause.as_ref() {
                let stats = velesdb_core::collection::search::query::match_planner::CollectionStats::default();
                velesdb_core::velesql::QueryPlan::from_match(match_clause, &stats)
            } else {
                velesdb_core::velesql::QueryPlan::from_select(&parsed.select)
            };

            let mut out = HashMap::new();
            out.insert("tree".to_string(), to_pyobject(py, plan.to_tree()));
            out.insert(
                "estimated_cost_ms".to_string(),
                to_pyobject(py, plan.estimated_cost_ms),
            );
            out.insert(
                "filter_strategy".to_string(),
                to_pyobject(py, plan.filter_strategy.as_str()),
            );
            out.insert(
                "index_used".to_string(),
                to_pyobject(py, plan.index_used.map(|i| i.as_str().to_string())),
            );
            Ok(out)
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
    // Index Management (EPIC-009 propagation)
    // ========================================================================

    /// Create a property index for O(1) equality lookups.
    #[pyo3(signature = (label, property))]
    fn create_property_index(&self, label: &str, property: &str) -> PyResult<()> {
        self.inner
            .create_property_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create property index: {e}")))
    }

    /// Create a range index for O(log n) range queries.
    #[pyo3(signature = (label, property))]
    fn create_range_index(&self, label: &str, property: &str) -> PyResult<()> {
        self.inner
            .create_range_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create range index: {e}")))
    }

    /// Check if a property index exists.
    #[pyo3(signature = (label, property))]
    fn has_property_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_property_index(label, property)
    }

    /// Check if a range index exists.
    #[pyo3(signature = (label, property))]
    fn has_range_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_range_index(label, property)
    }

    /// List all indexes on this collection.
    ///
    /// Returns:
    ///     List of dicts with keys: label, property, index_type, cardinality, memory_bytes
    ///
    /// Example:
    ///     >>> indexes = collection.list_indexes()
    ///     >>> for idx in indexes:
    ///     ...     print(f"{idx['label']}.{idx['property']} ({idx['index_type']})")
    fn list_indexes(&self) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let indexes = self.inner.list_indexes();
            let py_indexes: Vec<HashMap<String, PyObject>> = indexes
                .into_iter()
                .map(|idx| {
                    let mut result = HashMap::new();
                    result.insert("label".to_string(), to_pyobject(py, idx.label));
                    result.insert("property".to_string(), to_pyobject(py, idx.property));
                    result.insert("index_type".to_string(), to_pyobject(py, idx.index_type));
                    result.insert("cardinality".to_string(), to_pyobject(py, idx.cardinality));
                    result.insert(
                        "memory_bytes".to_string(),
                        to_pyobject(py, idx.memory_bytes),
                    );
                    result
                })
                .collect();
            Ok(py_indexes)
        })
    }

    /// Drop an index (either property or range).
    ///
    /// Args:
    ///     label: Node label
    ///     property: Property name
    ///
    /// Returns:
    ///     True if an index was dropped, False if no index existed
    ///
    /// Example:
    ///     >>> dropped = collection.drop_index("Person", "email")
    #[pyo3(signature = (label, property))]
    fn drop_index(&self, label: &str, property: &str) -> PyResult<bool> {
        self.inner
            .drop_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to drop index: {e}")))
    }

    // ========================================================================
    // VelesQL Query Execution (EPIC-056 US-002)
    // ========================================================================

    /// Execute a VelesQL query returning only IDs and scores (no payload).
    ///
    /// More efficient than `query()` when payload is not needed.
    ///
    /// Args:
    ///     velesql: VelesQL query string
    ///     params: Optional dict of query parameters
    ///
    /// Returns:
    ///     List of dicts with 'id' and 'score' fields
    ///
    /// Example:
    ///     >>> ids = collection.query_ids("SELECT * FROM docs WHERE price > 100 LIMIT 5")
    #[pyo3(signature = (velesql, params = None))]
    fn query_ids(
        &self,
        velesql: &str,
        params: Option<HashMap<String, PyObject>>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let parsed_query = velesdb_core::velesql::Parser::parse(velesql).map_err(|e| {
                PyRuntimeError::new_err(format!(
                    "VelesQL syntax error at position {}: {}",
                    e.position, e.message
                ))
            })?;

            let json_params: std::collections::HashMap<String, serde_json::Value> = params
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|json_val| (k, json_val)))
                .collect();

            let results = self
                .inner
                .execute_query(&parsed_query, &json_params)
                .map_err(|e| PyRuntimeError::new_err(format!("Query execution failed: {e}")))?;

            // Return only IDs and scores
            Ok(results
                .into_iter()
                .map(|r| {
                    let mut dict = HashMap::new();
                    dict.insert("id".to_string(), to_pyobject(py, r.point.id));
                    dict.insert("score".to_string(), to_pyobject(py, r.score));
                    dict
                })
                .collect())
        })
    }
}
