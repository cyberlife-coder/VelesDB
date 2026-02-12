//! Collection module for VelesDB Python bindings.
//!
//! This module contains the `Collection` struct and core CRUD methods.
//! Search, query, and graph methods are in separate modules:
//! - `collection_search.rs` — vector/text/hybrid search
//! - `collection_query.rs` — VelesQL query, MATCH, EXPLAIN
//! - `collection_graph.rs` — graph operations, index management

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::collection_helpers::point_to_dict;
use crate::utils::{extract_vector, python_to_json, to_pyobject};
use velesdb_core::Point;

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
}
