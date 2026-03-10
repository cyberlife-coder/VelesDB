//! Mutation methods for Collection (upsert, delete, flush, stream_insert).

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;
use velesdb_core::Point;

use crate::collection_helpers::parse_sparse_vectors_from_point;
use crate::utils::{extract_vector, python_to_json};

use super::Collection;

#[pymethods]
impl Collection {
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

                let payload = parse_payload(py, point_dict.get("payload"))?;
                let sparse_vectors = parse_sparse_vectors_from_point(py, &point_dict)?;
                core_points.push(Point::with_sparse(id, vector, payload, sparse_vectors));
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

                let payload = parse_payload(py, point_dict.get("payload"))?.ok_or_else(|| {
                    PyValueError::new_err("Metadata-only point must have 'payload' field")
                })?;

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

                let payload = parse_payload(py, point_dict.get("payload"))?;
                core_points.push(Point::new(id, vector, payload));
            }

            self.inner
                .upsert_bulk(&core_points)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to upsert_bulk: {e}")))
        })
    }

    /// Get points by their IDs.
    #[pyo3(signature = (ids))]
    fn get(&self, ids: Vec<u64>) -> PyResult<Vec<Option<HashMap<String, PyObject>>>> {
        Python::with_gil(|py| {
            let points = self.inner.get(&ids);
            let py_points = points
                .into_iter()
                .map(|opt_point| {
                    opt_point.map(|p| crate::collection_helpers::point_to_dict(py, &p))
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
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete: {e}")))
    }

    /// Flush all pending changes to disk.
    fn flush(&self) -> PyResult<()> {
        self.inner
            .flush()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to flush: {e}")))
    }

    /// Insert points via the streaming ingestion channel.
    ///
    /// Points are buffered and merged asynchronously into the HNSW index.
    /// This is faster than `upsert` for high-throughput workloads but offers
    /// eventual consistency (points appear in search after buffer merge).
    ///
    /// Args:
    ///     points: List of point dicts (same format as upsert).
    ///
    /// Returns:
    ///     Number of points successfully queued.
    ///
    /// Raises:
    ///     RuntimeError: If the streaming buffer is full or not configured.
    ///
    /// Example:
    ///     >>> count = collection.stream_insert([
    ///     ...     {"id": 1, "vector": [...], "payload": {"key": "value"}}
    ///     ... ])
    #[pyo3(signature = (points))]
    fn stream_insert(&self, points: Vec<HashMap<String, PyObject>>) -> PyResult<usize> {
        Python::with_gil(|py| {
            let mut count = 0usize;

            for point_dict in points {
                let id: u64 = point_dict
                    .get("id")
                    .ok_or_else(|| PyValueError::new_err("Point missing 'id' field"))?
                    .extract(py)?;

                let vector_obj = point_dict
                    .get("vector")
                    .ok_or_else(|| PyValueError::new_err("Point missing 'vector' field"))?;
                let vector = extract_vector(py, vector_obj)?;

                let payload = parse_payload(py, point_dict.get("payload"))?;
                let sparse_vectors = parse_sparse_vectors_from_point(py, &point_dict)?;
                let point = Point::with_sparse(id, vector, payload, sparse_vectors);

                self.inner.stream_insert(point).map_err(|e| {
                    PyRuntimeError::new_err(format!(
                        "Stream insert failed (buffer full or not configured): {e}"
                    ))
                })?;

                count += 1;
            }

            Ok(count)
        })
    }
}

/// Parse an optional payload PyObject into a JSON value.
fn parse_payload(
    py: Python<'_>,
    payload_obj: Option<&PyObject>,
) -> PyResult<Option<serde_json::Value>> {
    match payload_obj {
        Some(p) => {
            let dict: HashMap<String, PyObject> = p.extract(py).map_err(|_| {
                PyValueError::new_err("payload must be a dict[str, Any]")
            })?;
            let json_map: serde_json::Map<String, serde_json::Value> = dict
                .into_iter()
                .filter_map(|(k, v)| python_to_json(py, &v).map(|jv| (k, jv)))
                .collect();
            Ok(Some(serde_json::Value::Object(json_map)))
        }
        None => Ok(None),
    }
}
