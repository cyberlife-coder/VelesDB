//! Helper functions for Collection result conversions.
//!
//! Extracted from collection.rs to reduce file size and improve maintainability.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use std::collections::{BTreeMap, HashMap};

use crate::utils::{extract_vector, json_to_python, python_to_json};
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{Filter, Point, SearchResult};

/// Convert a `velesdb_core::Error` into a `PyRuntimeError`.
///
/// Shared helper that replaces ~39 inline `map_err(|e| PyRuntimeError::new_err(...))` closures
/// across the Python binding crate.
pub fn core_err(e: velesdb_core::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// Parse a Python filter object into a VelesDB Filter.
///
/// Converts the Python dict directly to `serde_json::Value` via [`python_to_json`],
/// then deserializes into [`Filter`]. This avoids the Python `json.dumps` round-trip.
pub fn parse_filter(py: Python<'_>, filter: &PyObject) -> PyResult<Filter> {
    let json_value = crate::utils::python_to_json(py, filter)?;
    Filter::from_json_value(json_value).map_err(|e| PyValueError::new_err(e))
}

/// Parse an optional Python filter object.
pub fn parse_optional_filter(py: Python<'_>, filter: Option<PyObject>) -> PyResult<Option<Filter>> {
    match filter {
        Some(f) => Ok(Some(parse_filter(py, &f)?)),
        None => Ok(None),
    }
}

/// Convert payload to Python object (shared helper to avoid duplication).
#[inline]
fn payload_to_python(py: Python<'_>, payload: &Option<serde_json::Value>) -> PyObject {
    match payload {
        Some(p) => json_to_python(py, p),
        None => py.None(),
    }
}

/// Convert a `SearchResult` to a Python dict, bypassing `HashMap` intermediary.
///
/// Uses `PyDict::new()` + `set_item()` directly and `PyString::intern()` for
/// static keys to avoid repeated string allocation.
pub fn search_result_to_dict(py: Python<'_>, result: &SearchResult) -> PyObject {
    let dict = PyDict::new(py);
    // PyString::intern reuses the same Python string object across calls
    let _ = dict.set_item(PyString::intern(py, "id"), result.point.id);
    let _ = dict.set_item(PyString::intern(py, "score"), result.score);
    let _ = dict.set_item(
        PyString::intern(py, "payload"),
        payload_to_python(py, &result.point.payload),
    );
    dict.into_any().unbind()
}

/// Convert a `SearchResult` to a multi-model Python dict (EPIC-031).
pub fn search_result_to_multimodel_dict(py: Python<'_>, result: &SearchResult) -> PyObject {
    let dict = PyDict::new(py);
    let none = py.None();
    let payload_py = payload_to_python(py, &result.point.payload);

    // Multi-model fields
    let _ = dict.set_item(PyString::intern(py, "node_id"), result.point.id);
    let _ = dict.set_item(PyString::intern(py, "vector_score"), result.score);
    let _ = dict.set_item(PyString::intern(py, "graph_score"), &none);
    let _ = dict.set_item(PyString::intern(py, "fused_score"), result.score);
    let _ = dict.set_item(PyString::intern(py, "bindings"), payload_py.clone_ref(py));
    let _ = dict.set_item(PyString::intern(py, "column_data"), &none);

    // Legacy fields for compatibility
    let _ = dict.set_item(PyString::intern(py, "id"), result.point.id);
    let _ = dict.set_item(PyString::intern(py, "score"), result.score);
    let _ = dict.set_item(PyString::intern(py, "payload"), payload_py);

    dict.into_any().unbind()
}

/// Convert a `Point` to a Python dict.
pub fn point_to_dict(py: Python<'_>, point: &Point) -> PyObject {
    let dict = PyDict::new(py);
    let _ = dict.set_item(PyString::intern(py, "id"), point.id);
    let np_vector = numpy::PyArray1::from_slice(py, &point.vector);
    let _ = dict.set_item(PyString::intern(py, "vector"), np_vector);
    let _ = dict.set_item(
        PyString::intern(py, "payload"),
        payload_to_python(py, &point.payload),
    );
    dict.into_any().unbind()
}

/// Convert a list of `SearchResult`s to Python dicts.
pub fn search_results_to_dicts(py: Python<'_>, results: Vec<SearchResult>) -> Vec<PyObject> {
    results
        .into_iter()
        .map(|r| search_result_to_dict(py, &r))
        .collect()
}

/// Convert a list of `SearchResult`s to multi-model Python dicts.
pub fn search_results_to_multimodel_dicts(
    py: Python<'_>,
    results: Vec<SearchResult>,
) -> Vec<PyObject> {
    results
        .into_iter()
        .map(|r| search_result_to_multimodel_dict(py, &r))
        .collect()
}

/// Convert a list of (id, score) pairs to Python dicts.
pub fn id_score_pairs_to_dicts(py: Python<'_>, results: Vec<(u64, f32)>) -> Vec<PyObject> {
    results
        .into_iter()
        .map(|(id, score)| {
            let dict = PyDict::new(py);
            let _ = dict.set_item(PyString::intern(py, "id"), id);
            let _ = dict.set_item(PyString::intern(py, "score"), score);
            dict.into_any().unbind()
        })
        .collect()
}

/// Parse a Python object into a `SparseVector`.
///
/// Accepts:
/// - A Python `dict[int, float]` mapping dimension indices to weights.
/// - A scipy sparse object with a `.toarray()` method (COO/CSR/CSC).
///
/// Returns `PyValueError` if the object is neither format.
pub fn parse_sparse_vector(py: Python<'_>, obj: &PyObject) -> PyResult<SparseVector> {
    // Try dict[int, float] first (most common usage pattern).
    if let Ok(dict) = obj.extract::<HashMap<u32, f32>>(py) {
        let pairs: Vec<(u32, f32)> = dict.into_iter().collect();
        return Ok(SparseVector::new(pairs));
    }

    // Try scipy.sparse via duck typing: check for `.toarray()` method.
    // First confirm the attribute exists without calling it, so we can distinguish
    // "not a scipy object" (AttributeError → try next format) from runtime errors.
    let has_toarray_attr = obj
        .getattr(py, "toarray")
        .map(|attr| !attr.is_none(py))
        .unwrap_or(false);

    if has_toarray_attr {
        // `.toarray()` returns a dense numpy 2D array; flatten to 1D.
        let array = obj
            .call_method0(py, "toarray")
            .map_err(|e| PyValueError::new_err(format!("scipy toarray() failed: {e}")))?;
        let flat = array
            .call_method0(py, "flatten")
            .map_err(|e| PyValueError::new_err(format!("scipy flatten() failed: {e}")))?;
        let values: Vec<f32> = flat.extract(py).map_err(|e| {
            PyValueError::new_err(format!("Failed to extract floats from scipy array: {e}"))
        })?;
        let pairs: Vec<(u32, f32)> = values
            .into_iter()
            .enumerate()
            .filter(|(_, v)| v.abs() > f32::EPSILON)
            .map(|(i, v)| u32::try_from(i).map(|idx| (idx, v)))
            .collect::<Result<_, _>>()
            .map_err(|_| {
                PyValueError::new_err("Sparse vector has more than u32::MAX dimensions")
            })?;
        return Ok(SparseVector::new(pairs));
    }

    Err(PyValueError::new_err(
        "sparse_vector must be a dict[int, float] or a scipy sparse object with .toarray()",
    ))
}

/// Parse the `sparse_vector` field from a point dict into named sparse vectors.
///
/// Accepts:
/// - `dict[int, float]`: treated as the default (unnamed) sparse vector.
/// - `dict[str, dict[int, float]]`: named sparse vectors.
///
/// Returns `None` if the key is absent from the point dict.
pub fn parse_sparse_vectors_from_point(
    py: Python<'_>,
    point_dict: &HashMap<String, PyObject>,
) -> PyResult<Option<BTreeMap<String, SparseVector>>> {
    let Some(obj) = point_dict.get("sparse_vector") else {
        return Ok(None);
    };

    // Try as dict[int, float] -> default sparse vector (key "").
    if let Ok(dict) = obj.extract::<HashMap<u32, f32>>(py) {
        let sv = SparseVector::new(dict.into_iter().collect());
        let mut map = BTreeMap::new();
        map.insert(String::new(), sv);
        return Ok(Some(map));
    }

    // Try as dict[str, dict[int, float]] -> named sparse vectors.
    if let Ok(named) = obj.extract::<HashMap<String, PyObject>>(py) {
        let mut map = BTreeMap::new();
        for (name, inner_obj) in named {
            let sv = parse_sparse_vector(py, &inner_obj)?;
            map.insert(name, sv);
        }
        return Ok(Some(map));
    }

    Err(PyValueError::new_err(
        "sparse_vector must be dict[int, float] or dict[str, dict[int, float]]",
    ))
}

// ---------------------------------------------------------------------------
// Point dict parsing helpers (DRY-02, DRY-03, FOWL-04)
// ---------------------------------------------------------------------------

/// Convert a Python `dict[str, Any]` to a `serde_json::Value::Object`.
///
/// Shared helper used by both `upsert_bulk_numpy`'s payload loop and
/// `parse_payload` to avoid duplicating the dict-to-JSON conversion.
pub fn dict_to_json_map(
    py: Python<'_>,
    dict: &HashMap<String, PyObject>,
) -> PyResult<serde_json::Value> {
    let json_map: serde_json::Map<String, serde_json::Value> = dict
        .iter()
        .map(|(k, v)| python_to_json(py, v).map(|jv| (k.clone(), jv)))
        .collect::<PyResult<_>>()?;
    Ok(serde_json::Value::Object(json_map))
}

/// Extract the `"id"` field from a point dict as `u64`.
pub fn extract_point_id(py: Python<'_>, dict: &HashMap<String, PyObject>) -> PyResult<u64> {
    dict.get("id")
        .ok_or_else(|| PyValueError::new_err("Point missing 'id' field"))?
        .extract(py)
}

/// Extract the `"vector"` field from a point dict as `Vec<f32>`.
pub fn extract_point_vector(
    py: Python<'_>,
    dict: &HashMap<String, PyObject>,
) -> PyResult<Vec<f32>> {
    let obj = dict
        .get("vector")
        .ok_or_else(|| PyValueError::new_err("Point missing 'vector' field"))?;
    extract_vector(py, obj)
}

/// Parse a list of point dicts into core `Point` values.
///
/// Each dict must contain `"id"` (u64) and `"vector"` (list/numpy) keys,
/// with optional `"payload"` (dict) and `"sparse_vector"` entries.
///
/// Shared by [`Collection::upsert`] and [`Collection::stream_insert`].
pub fn parse_point_dicts(
    py: Python<'_>,
    points: &[HashMap<String, PyObject>],
) -> PyResult<Vec<Point>> {
    let mut result = Vec::with_capacity(points.len());
    for point_dict in points {
        let id = extract_point_id(py, point_dict)?;
        let vector = extract_point_vector(py, point_dict)?;
        let payload = parse_payload(py, point_dict.get("payload"))?;
        let sparse_vectors = parse_sparse_vectors_from_point(py, point_dict)?;
        result.push(Point::with_sparse(id, vector, payload, sparse_vectors));
    }
    Ok(result)
}

/// Parse an optional payload `PyObject` into a JSON value.
pub fn parse_payload(
    py: Python<'_>,
    payload_obj: Option<&PyObject>,
) -> PyResult<Option<serde_json::Value>> {
    match payload_obj {
        Some(p) => {
            let dict: HashMap<String, PyObject> = p
                .extract(py)
                .map_err(|_| PyValueError::new_err("payload must be a dict[str, Any]"))?;
            Ok(Some(dict_to_json_map(py, &dict)?))
        }
        None => Ok(None),
    }
}
