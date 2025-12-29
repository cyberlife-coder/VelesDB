#![allow(clippy::useless_conversion)]
#![allow(clippy::pedantic)] // PyO3 binding code has many style differences
#![allow(deprecated)] // PyO3 0.24 deprecation warnings - will migrate to IntoPyObject in future
//! Python bindings for `VelesDB` vector database.
//!
//! This module provides a Pythonic interface to VelesDB using PyO3.
//!
//! # Example
//!
//! ```python
//! import velesdb
//!
//! # Open database
//! db = velesdb.Database("./my_data")
//!
//! # Create collection
//! collection = db.create_collection("documents", dimension=768, metric="cosine")
//!
//! # Insert vectors
//! collection.upsert([
//!     {"id": 1, "vector": [0.1, 0.2, ...], "payload": {"title": "Doc 1"}}
//! ])
//!
//! # Search
//! results = collection.search([0.1, 0.2, ...], top_k=10)
//! ```

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use velesdb_core::{Database as CoreDatabase, DistanceMetric, Point};

/// Extracts a vector from a PyObject, supporting both Python lists and NumPy arrays.
///
/// # Arguments
/// * `py` - Python GIL token
/// * `obj` - The Python object (list or numpy.ndarray)
///
/// # Returns
/// A Vec<f32> containing the vector data
///
/// # Errors
/// Returns an error if the object is neither a list nor a numpy array
fn extract_vector(py: Python<'_>, obj: &PyObject) -> PyResult<Vec<f32>> {
    // Try numpy array first (most common in ML workflows)
    if let Ok(array) = obj.extract::<numpy::PyReadonlyArray1<f32>>(py) {
        return Ok(array.as_slice()?.to_vec());
    }

    // Try numpy float64 array and convert
    if let Ok(array) = obj.extract::<numpy::PyReadonlyArray1<f64>>(py) {
        return Ok(array.as_slice()?.iter().map(|&x| x as f32).collect());
    }

    // Fall back to Python list
    if let Ok(list) = obj.extract::<Vec<f32>>(py) {
        return Ok(list);
    }

    Err(PyValueError::new_err(
        "Vector must be a Python list or numpy array of floats",
    ))
}

/// VelesDB Database - the main entry point for interacting with VelesDB.
///
/// Example:
///     >>> db = velesdb.Database("./my_data")
///     >>> collections = db.list_collections()
#[pyclass]
pub struct Database {
    inner: CoreDatabase,
}

#[pymethods]
impl Database {
    /// Create or open a VelesDB database at the specified path.
    ///
    /// Args:
    ///     path: Directory path for database storage
    ///
    /// Returns:
    ///     Database instance
    ///
    /// Example:
    ///     >>> db = velesdb.Database("./my_vectors")
    #[new]
    #[pyo3(signature = (path))]
    fn new(path: &str) -> PyResult<Self> {
        let db = CoreDatabase::open(PathBuf::from(path))
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to open database: {}", e)))?;
        Ok(Self { inner: db })
    }

    /// Create a new vector collection.
    ///
    /// Args:
    ///     name: Collection name
    ///     dimension: Vector dimension (e.g., 768 for BERT embeddings)
    ///     metric: Distance metric - "cosine", "euclidean", "dot", "hamming", or "jaccard"
    ///             (default: "cosine")
    ///
    /// Returns:
    ///     Collection instance
    ///
    /// Example:
    ///     >>> collection = db.create_collection("documents", dimension=768, metric="cosine")
    ///     >>> # For binary vectors:
    ///     >>> fingerprints = db.create_collection("hashes", dimension=256, metric="hamming")
    #[pyo3(signature = (name, dimension, metric = "cosine"))]
    fn create_collection(
        &self,
        name: &str,
        dimension: usize,
        metric: &str,
    ) -> PyResult<Collection> {
        let distance_metric = parse_metric(metric)?;

        self.inner
            .create_collection(name, dimension, distance_metric)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create collection: {}", e)))?;

        let collection = self
            .inner
            .get_collection(name)
            .ok_or_else(|| PyRuntimeError::new_err("Collection not found after creation"))?;

        Ok(Collection {
            inner: Arc::new(collection),
            name: name.to_string(),
        })
    }

    /// Get an existing collection by name.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     Collection instance or None if not found
    ///
    /// Example:
    ///     >>> collection = db.get_collection("documents")
    #[pyo3(signature = (name))]
    fn get_collection(&self, name: &str) -> PyResult<Option<Collection>> {
        match self.inner.get_collection(name) {
            Some(collection) => Ok(Some(Collection {
                inner: Arc::new(collection),
                name: name.to_string(),
            })),
            None => Ok(None),
        }
    }

    /// List all collection names in the database.
    ///
    /// Returns:
    ///     List of collection names
    ///
    /// Example:
    ///     >>> names = db.list_collections()
    ///     >>> print(names)  # ['documents', 'images']
    fn list_collections(&self) -> Vec<String> {
        self.inner.list_collections()
    }

    /// Delete a collection by name.
    ///
    /// Args:
    ///     name: Collection name to delete
    ///
    /// Example:
    ///     >>> db.delete_collection("old_collection")
    #[pyo3(signature = (name))]
    fn delete_collection(&self, name: &str) -> PyResult<()> {
        self.inner
            .delete_collection(name)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete collection: {}", e)))
    }
}

/// A vector collection in VelesDB.
///
/// Collections store vectors with optional metadata (payload) and support
/// efficient similarity search.
#[pyclass]
pub struct Collection {
    inner: Arc<velesdb_core::Collection>,
    name: String,
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
    ///     Dict with name, dimension, metric, and point_count
    fn info(&self) -> PyResult<HashMap<String, PyObject>> {
        Python::with_gil(|py| {
            let config = self.inner.config();
            let mut info = HashMap::new();
            info.insert("name".to_string(), config.name.into_py(py));
            info.insert("dimension".to_string(), config.dimension.into_py(py));
            info.insert(
                "metric".to_string(),
                format!("{:?}", config.metric).to_lowercase().into_py(py),
            );
            info.insert("point_count".to_string(), config.point_count.into_py(py));
            Ok(info)
        })
    }

    /// Insert or update vectors in the collection.
    ///
    /// Args:
    ///     points: List of point dicts with 'id', 'vector', and optional 'payload'
    ///
    /// Example:
    ///     >>> collection.upsert([
    ///     ...     {"id": 1, "vector": [0.1, 0.2, 0.3], "payload": {"title": "Doc 1"}},
    ///     ...     {"id": 2, "vector": [0.4, 0.5, 0.6], "payload": {"title": "Doc 2"}}
    ///     ... ])
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

                        // Try to parse as JSON, otherwise create a simple object
                        if let Ok(json_val) = serde_json::from_str(&payload_str) {
                            Some(json_val)
                        } else {
                            // Convert Python dict to JSON
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
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to upsert: {}", e)))?;

            Ok(count)
        })
    }

    /// Bulk insert optimized for high-throughput import.
    ///
    /// This method is ~2-3x faster than regular upsert() for large batches:
    /// - Uses parallel HNSW insertion (rayon)
    /// - Single flush at the end (not per-batch)
    ///
    /// Args:
    ///     points: List of point dicts with 'id', 'vector', and optional 'payload'
    ///
    /// Example:
    ///     >>> # For bulk loading, use upsert_bulk instead of upsert
    ///     >>> collection.upsert_bulk([
    ///     ...     {"id": i, "vector": vectors[i]} for i in range(10000)
    ///     ... ])
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
    ///
    /// Args:
    ///     vector: Query vector (Python list or numpy array)
    ///     top_k: Number of results to return (default: 10)
    ///
    /// Returns:
    ///     List of search results with 'id', 'score', and 'payload'
    ///
    /// Example:
    ///     >>> results = collection.search([0.1, 0.2, 0.3], top_k=5)
    ///     >>> # Or with numpy:
    ///     >>> results = collection.search(np.array([0.1, 0.2, 0.3]), top_k=5)
    ///     >>> for r in results:
    ///     ...     print(f"ID: {r['id']}, Score: {r['score']}")
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
                    result.insert("id".to_string(), r.point.id.into_py(py));
                    result.insert("score".to_string(), r.score.into_py(py));

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
    ///
    /// Args:
    ///     ids: List of point IDs to retrieve
    ///
    /// Returns:
    ///     List of points (or None for missing IDs)
    ///
    /// Example:
    ///     >>> points = collection.get([1, 2, 3])
    #[pyo3(signature = (ids))]
    fn get(&self, ids: Vec<u64>) -> PyResult<Vec<Option<HashMap<String, PyObject>>>> {
        Python::with_gil(|py| {
            let points = self.inner.get(&ids);

            let py_points: Vec<Option<HashMap<String, PyObject>>> = points
                .into_iter()
                .map(|opt_point| {
                    opt_point.map(|p| {
                        let mut result = HashMap::new();
                        result.insert("id".to_string(), p.id.into_py(py));
                        result.insert("vector".to_string(), p.vector.into_py(py));

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
    ///
    /// Args:
    ///     ids: List of point IDs to delete
    ///
    /// Example:
    ///     >>> collection.delete([1, 2, 3])
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
}

/// Search result from a vector query.
#[pyclass]
pub struct SearchResult {
    #[pyo3(get)]
    id: u64,
    #[pyo3(get)]
    score: f32,
    #[pyo3(get)]
    payload: PyObject,
}

// Helper functions

fn parse_metric(metric: &str) -> PyResult<DistanceMetric> {
    match metric.to_lowercase().as_str() {
        "cosine" => Ok(DistanceMetric::Cosine),
        "euclidean" | "l2" => Ok(DistanceMetric::Euclidean),
        "dot" | "dotproduct" | "ip" => Ok(DistanceMetric::DotProduct),
        "hamming" => Ok(DistanceMetric::Hamming),
        "jaccard" => Ok(DistanceMetric::Jaccard),
        _ => Err(PyValueError::new_err(format!(
            "Invalid metric '{}'. Use 'cosine', 'euclidean', 'dot', 'hamming', or 'jaccard'",
            metric
        ))),
    }
}

fn python_to_json(py: Python<'_>, obj: &PyObject) -> Option<serde_json::Value> {
    if let Ok(s) = obj.extract::<String>(py) {
        return Some(serde_json::Value::String(s));
    }
    if let Ok(i) = obj.extract::<i64>(py) {
        return Some(serde_json::Value::Number(i.into()));
    }
    if let Ok(f) = obj.extract::<f64>(py) {
        return serde_json::Number::from_f64(f).map(serde_json::Value::Number);
    }
    if let Ok(b) = obj.extract::<bool>(py) {
        return Some(serde_json::Value::Bool(b));
    }
    if obj.is_none(py) {
        return Some(serde_json::Value::Null);
    }
    if let Ok(list) = obj.extract::<Vec<PyObject>>(py) {
        let arr: Vec<serde_json::Value> = list
            .iter()
            .filter_map(|item| python_to_json(py, item))
            .collect();
        return Some(serde_json::Value::Array(arr));
    }
    if let Ok(dict) = obj.extract::<HashMap<String, PyObject>>(py) {
        let map: serde_json::Map<String, serde_json::Value> = dict
            .into_iter()
            .filter_map(|(k, v)| python_to_json(py, &v).map(|jv| (k, jv)))
            .collect();
        return Some(serde_json::Value::Object(map));
    }
    None
}

fn json_to_python(py: Python<'_>, value: &serde_json::Value) -> PyObject {
    match value {
        serde_json::Value::Null => py.None(),
        serde_json::Value::Bool(b) => b.into_py(py),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_py(py)
            } else if let Some(f) = n.as_f64() {
                f.into_py(py)
            } else {
                py.None()
            }
        }
        serde_json::Value::String(s) => s.into_py(py),
        serde_json::Value::Array(arr) => {
            let list: Vec<PyObject> = arr.iter().map(|v| json_to_python(py, v)).collect();
            list.into_py(py)
        }
        serde_json::Value::Object(map) => {
            let dict: HashMap<String, PyObject> = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_python(py, v)))
                .collect();
            dict.into_py(py)
        }
    }
}

/// VelesDB - A high-performance vector database for AI applications.
///
/// Example:
///     >>> import velesdb
///     >>> db = velesdb.Database("./my_data")
///     >>> collection = db.create_collection("docs", dimension=768)
///     >>> collection.upsert([{"id": 1, "vector": [...], "payload": {"title": "Doc"}}])
///     >>> results = collection.search([...], top_k=10)
#[pymodule]
fn velesdb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Database>()?;
    m.add_class::<Collection>()?;
    m.add_class::<SearchResult>()?;

    // Add version info
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    Ok(())
}
