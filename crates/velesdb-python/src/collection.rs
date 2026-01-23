//! Collection module for VelesDB Python bindings.
//!
//! This module contains the `Collection` struct and all its PyO3 methods
//! for vector storage and similarity search operations.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

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

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut result = HashMap::new();
                    result.insert("id".to_string(), to_pyobject(py, r.point.id));
                    result.insert("score".to_string(), to_pyobject(py, r.score));

                    let payload_py = match &r.point.payload {
                        Some(p) => json_to_python(py, p),
                        None => py.None(),
                    };
                    result.insert("payload".to_string(), payload_py);

                    result
                })
                .collect();

            Ok(py_results)
        })
    }

    /// Get points by their IDs.
    #[pyo3(signature = (ids))]
    fn get(&self, ids: Vec<u64>) -> PyResult<Vec<Option<HashMap<String, PyObject>>>> {
        Python::with_gil(|py| {
            let points = self.inner.get(&ids);

            let py_points: Vec<Option<HashMap<String, PyObject>>> = points
                .into_iter()
                .map(|opt_point| {
                    opt_point.map(|p| {
                        let mut result = HashMap::new();
                        result.insert("id".to_string(), to_pyobject(py, p.id));
                        result.insert("vector".to_string(), to_pyobject(py, p.vector.clone()));

                        let payload_py = match &p.payload {
                            Some(payload) => json_to_python(py, payload),
                            None => py.None(),
                        };
                        result.insert("payload".to_string(), payload_py);

                        result
                    })
                })
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
            let filter_obj = match filter {
                Some(f) => {
                    let json_str = py
                        .import("json")?
                        .call_method1("dumps", (f,))?
                        .extract::<String>()?;
                    let filter: velesdb_core::Filter = serde_json::from_str(&json_str)
                        .map_err(|e| PyValueError::new_err(format!("Invalid filter: {e}")))?;
                    Some(filter)
                }
                None => None,
            };

            let results = if let Some(f) = filter_obj {
                self.inner.text_search_with_filter(query, top_k, &f)
            } else {
                self.inner.text_search(query, top_k)
            };

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut result = HashMap::new();
                    result.insert("id".to_string(), to_pyobject(py, r.point.id));
                    result.insert("score".to_string(), to_pyobject(py, r.score));

                    let payload_py = match &r.point.payload {
                        Some(p) => json_to_python(py, p),
                        None => py.None(),
                    };
                    result.insert("payload".to_string(), payload_py);

                    result
                })
                .collect();

            Ok(py_results)
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

            let filter_obj = match filter {
                Some(f) => {
                    let json_str = py
                        .import("json")?
                        .call_method1("dumps", (f,))?
                        .extract::<String>()?;
                    let filter: velesdb_core::Filter = serde_json::from_str(&json_str)
                        .map_err(|e| PyValueError::new_err(format!("Invalid filter: {e}")))?;
                    Some(filter)
                }
                None => None,
            };

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

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut result = HashMap::new();
                    result.insert("id".to_string(), to_pyobject(py, r.point.id));
                    result.insert("score".to_string(), to_pyobject(py, r.score));

                    let payload_py = match &r.point.payload {
                        Some(p) => json_to_python(py, p),
                        None => py.None(),
                    };
                    result.insert("payload".to_string(), payload_py);

                    result
                })
                .collect();

            Ok(py_results)
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
                let vector = extract_vector(py, vector_obj)?;
                queries.push(vector);

                let top_k: usize = search_dict
                    .get("top_k")
                    .or_else(|| search_dict.get("topK"))
                    .map(|v| v.extract(py))
                    .transpose()?
                    .unwrap_or(10);
                top_ks.push(top_k);

                let filter_obj = match search_dict.get("filter") {
                    Some(f) => {
                        let json_str = py
                            .import("json")?
                            .call_method1("dumps", (f,))?
                            .extract::<String>()?;
                        let filter: velesdb_core::Filter = serde_json::from_str(&json_str)
                            .map_err(|e| PyValueError::new_err(format!("Invalid filter: {e}")))?;
                        Some(filter)
                    }
                    None => None,
                };
                filters.push(filter_obj);
            }

            let max_top_k = top_ks.iter().max().copied().unwrap_or(10);
            let query_refs: Vec<&[f32]> = queries.iter().map(|v| v.as_slice()).collect();

            let batch_results = self
                .inner
                .search_batch_with_filters(&query_refs, max_top_k, &filters)
                .map_err(|e| PyRuntimeError::new_err(format!("Batch search failed: {e}")))?;

            let py_batch_results: Vec<Vec<HashMap<String, PyObject>>> = batch_results
                .into_iter()
                .zip(top_ks)
                .map(|(results, k)| {
                    results
                        .into_iter()
                        .take(k)
                        .map(|r| {
                            let mut result = HashMap::new();
                            result.insert("id".to_string(), to_pyobject(py, r.point.id));
                            result.insert("score".to_string(), to_pyobject(py, r.score));

                            let payload_py = match &r.point.payload {
                                Some(p) => json_to_python(py, p),
                                None => py.None(),
                            };
                            result.insert("payload".to_string(), payload_py);

                            result
                        })
                        .collect()
                })
                .collect();

            Ok(py_batch_results)
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

            let filter_obj = match filter {
                Some(f) => {
                    let json_str = py
                        .import("json")?
                        .call_method1("dumps", (f,))?
                        .extract::<String>()?;
                    let filter: velesdb_core::Filter = serde_json::from_str(&json_str)
                        .map_err(|e| PyValueError::new_err(format!("Invalid filter: {e}")))?;
                    filter
                }
                None => {
                    return Err(PyValueError::new_err(
                        "Filter is required for search_with_filter",
                    ));
                }
            };

            let results = self
                .inner
                .search_with_filter(&query_vector, top_k, &filter_obj)
                .map_err(|e| PyRuntimeError::new_err(format!("Search with filter failed: {e}")))?;

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut result = HashMap::new();
                    result.insert("id".to_string(), to_pyobject(py, r.point.id));
                    result.insert("score".to_string(), to_pyobject(py, r.score));

                    let payload_py = match &r.point.payload {
                        Some(p) => json_to_python(py, p),
                        None => py.None(),
                    };
                    result.insert("payload".to_string(), payload_py);

                    result
                })
                .collect();

            Ok(py_results)
        })
    }

    /// Execute a VelesQL query.
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

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut result = HashMap::new();
                    result.insert("id".to_string(), to_pyobject(py, r.point.id));
                    result.insert("score".to_string(), to_pyobject(py, r.score));

                    let payload_py = match &r.point.payload {
                        Some(p) => json_to_python(py, p),
                        None => py.None(),
                    };
                    result.insert("payload".to_string(), payload_py);

                    result
                })
                .collect();

            Ok(py_results)
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
            let mut query_vectors: Vec<Vec<f32>> = Vec::with_capacity(vectors.len());
            for v in &vectors {
                query_vectors.push(extract_vector(py, v)?);
            }

            let fusion_strategy = fusion
                .map(|f| f.inner())
                .unwrap_or(CoreFusionStrategy::RRF { k: 60 });

            let filter_obj = match filter {
                Some(f) => {
                    let json_str = py
                        .import("json")?
                        .call_method1("dumps", (f,))?
                        .extract::<String>()?;
                    let filter: velesdb_core::Filter = serde_json::from_str(&json_str)
                        .map_err(|e| PyValueError::new_err(format!("Invalid filter: {e}")))?;
                    Some(filter)
                }
                None => None,
            };

            let query_refs: Vec<&[f32]> = query_vectors.iter().map(|v| v.as_slice()).collect();

            let results = self
                .inner
                .multi_query_search(&query_refs, top_k, fusion_strategy, filter_obj.as_ref())
                .map_err(|e| PyRuntimeError::new_err(format!("Multi-query search failed: {e}")))?;

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|r| {
                    let mut result = HashMap::new();
                    result.insert("id".to_string(), to_pyobject(py, r.point.id));
                    result.insert("score".to_string(), to_pyobject(py, r.score));

                    let payload_py = match &r.point.payload {
                        Some(p) => json_to_python(py, p),
                        None => py.None(),
                    };
                    result.insert("payload".to_string(), payload_py);

                    result
                })
                .collect();

            Ok(py_results)
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
            let mut query_vectors: Vec<Vec<f32>> = Vec::with_capacity(vectors.len());
            for v in &vectors {
                query_vectors.push(extract_vector(py, v)?);
            }

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

            let py_results: Vec<HashMap<String, PyObject>> = results
                .into_iter()
                .map(|(id, score)| {
                    let mut result = HashMap::new();
                    result.insert("id".to_string(), to_pyobject(py, id));
                    result.insert("score".to_string(), to_pyobject(py, score));
                    result
                })
                .collect();

            Ok(py_results)
        })
    }

    // ========================================================================
    // Index Management (EPIC-009 propagation)
    // ========================================================================

    /// Create a property index for O(1) equality lookups on graph nodes.
    ///
    /// Args:
    ///     label: Node label to index (e.g., "Person")
    ///     property: Property name to index (e.g., "email")
    ///
    /// Example:
    ///     >>> collection.create_property_index("Person", "email")
    #[pyo3(signature = (label, property))]
    fn create_property_index(&self, label: &str, property: &str) -> PyResult<()> {
        self.inner
            .create_property_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create property index: {e}")))
    }

    /// Create a range index for O(log n) range queries on graph nodes.
    ///
    /// Args:
    ///     label: Node label to index (e.g., "Event")
    ///     property: Property name to index (e.g., "timestamp")
    ///
    /// Example:
    ///     >>> collection.create_range_index("Event", "timestamp")
    #[pyo3(signature = (label, property))]
    fn create_range_index(&self, label: &str, property: &str) -> PyResult<()> {
        self.inner
            .create_range_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create range index: {e}")))
    }

    /// Check if a property index exists.
    ///
    /// Args:
    ///     label: Node label
    ///     property: Property name
    ///
    /// Returns:
    ///     True if a property index exists for this label/property combination
    #[pyo3(signature = (label, property))]
    fn has_property_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_property_index(label, property)
    }

    /// Check if a range index exists.
    ///
    /// Args:
    ///     label: Node label
    ///     property: Property name
    ///
    /// Returns:
    ///     True if a range index exists for this label/property combination
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
}
