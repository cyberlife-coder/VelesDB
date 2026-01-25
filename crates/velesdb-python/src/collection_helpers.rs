//! Helper functions for Collection result conversions.
//!
//! Extracted from collection.rs to reduce file size and improve maintainability.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::utils::{json_to_python, to_pyobject};
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

    // Payload as bindings
    let bindings_py = match &result.point.payload {
        Some(p) => json_to_python(py, p),
        None => py.None(),
    };
    dict.insert("bindings".to_string(), bindings_py);
    dict.insert("column_data".to_string(), py.None());

    // Legacy fields for compatibility
    dict.insert("id".to_string(), to_pyobject(py, result.point.id));
    dict.insert("score".to_string(), to_pyobject(py, result.score));
    let payload_py = match &result.point.payload {
        Some(p) => json_to_python(py, p),
        None => py.None(),
    };
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
