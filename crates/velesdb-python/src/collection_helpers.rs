//! Helper functions for Collection result conversions.
//!
//! Extracted from collection.rs to reduce file size and improve maintainability.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::{BTreeMap, HashMap};

use crate::utils::{json_to_python, to_pyobject};
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{Filter, Point, SearchResult};

/// Parse a Python filter object into a VelesDB Filter.
pub fn parse_filter(py: Python<'_>, filter: &PyObject) -> PyResult<Filter> {
    let json_str = py
        .import("json")?
        .call_method1("dumps", (filter,))?
        .extract::<String>()?;
    serde_json::from_str(&json_str)
        .map_err(|e| PyValueError::new_err(format!("Invalid filter: {e}")))
}

/// Parse an optional Python filter object.
pub fn parse_optional_filter(py: Python<'_>, filter: Option<PyObject>) -> PyResult<Option<Filter>> {
    match filter {
        Some(f) => Ok(Some(parse_filter(py, &f)?)),
        None => Ok(None),
    }
}

/// Convert a SearchResult to a Python dictionary.
pub fn search_result_to_dict(py: Python<'_>, result: &SearchResult) -> HashMap<String, PyObject> {
    let mut dict = HashMap::new();
    dict.insert("id".to_string(), to_pyobject(py, result.point.id));
    dict.insert("score".to_string(), to_pyobject(py, result.score));

    let payload_py = match &result.point.payload {
        Some(p) => json_to_python(py, p),
        None => py.None(),
    };
    dict.insert("payload".to_string(), payload_py);

    dict
}

/// Convert a SearchResult to a multi-model Python dictionary (EPIC-031).
pub fn search_result_to_multimodel_dict(
    py: Python<'_>,
    result: &SearchResult,
) -> HashMap<String, PyObject> {
    let mut dict = HashMap::new();

    // Multi-model fields
    dict.insert("node_id".to_string(), to_pyobject(py, result.point.id));
    dict.insert("vector_score".to_string(), to_pyobject(py, result.score));
    dict.insert("graph_score".to_string(), py.None());
    dict.insert("fused_score".to_string(), to_pyobject(py, result.score));

    // Payload as bindings — convert once, then clone the reference for the legacy field.
    let bindings_py = match &result.point.payload {
        Some(p) => json_to_python(py, p),
        None => py.None(),
    };
    let payload_py = bindings_py.clone_ref(py);
    dict.insert("bindings".to_string(), bindings_py);
    dict.insert("column_data".to_string(), py.None());

    // Legacy fields for compatibility
    dict.insert("id".to_string(), to_pyobject(py, result.point.id));
    dict.insert("score".to_string(), to_pyobject(py, result.score));
    dict.insert("payload".to_string(), payload_py);

    dict
}

/// Convert a Point to a Python dictionary.
pub fn point_to_dict(py: Python<'_>, point: &Point) -> HashMap<String, PyObject> {
    let mut dict = HashMap::new();
    dict.insert("id".to_string(), to_pyobject(py, point.id));
    dict.insert("vector".to_string(), to_pyobject(py, point.vector.clone()));

    let payload_py = match &point.payload {
        Some(p) => json_to_python(py, p),
        None => py.None(),
    };
    dict.insert("payload".to_string(), payload_py);

    dict
}

/// Convert a list of SearchResults to Python dictionaries.
pub fn search_results_to_dicts(
    py: Python<'_>,
    results: Vec<SearchResult>,
) -> Vec<HashMap<String, PyObject>> {
    results
        .into_iter()
        .map(|r| search_result_to_dict(py, &r))
        .collect()
}

/// Convert a list of SearchResults to multi-model Python dictionaries.
pub fn search_results_to_multimodel_dicts(
    py: Python<'_>,
    results: Vec<SearchResult>,
) -> Vec<HashMap<String, PyObject>> {
    results
        .into_iter()
        .map(|r| search_result_to_multimodel_dict(py, &r))
        .collect()
}

/// Convert a list of (id, score) pairs to Python dictionaries.
pub fn id_score_pairs_to_dicts(
    py: Python<'_>,
    results: Vec<(u64, f32)>,
) -> Vec<HashMap<String, PyObject>> {
    results
        .into_iter()
        .map(|(id, score)| {
            let mut dict = HashMap::new();
            dict.insert("id".to_string(), to_pyobject(py, id));
            dict.insert("score".to_string(), to_pyobject(py, score));
            dict
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
