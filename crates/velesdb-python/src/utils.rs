//! Utility functions for Python-Rust type conversions.
//!
//! This module provides helper functions for converting between Python and Rust types,
//! particularly for JSON serialization and distance metric/storage mode parsing.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::IntoPyObjectExt;
use std::collections::HashMap;
use velesdb_core::{DistanceMetric, StorageMode};

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
pub fn extract_vector(py: Python<'_>, obj: &PyObject) -> PyResult<Vec<f32>> {
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

/// Parse a distance metric string into a DistanceMetric enum.
pub fn parse_metric(metric: &str) -> PyResult<DistanceMetric> {
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

/// Parse a storage mode string into a StorageMode enum.
pub fn parse_storage_mode(mode: &str) -> PyResult<StorageMode> {
    match mode.to_lowercase().as_str() {
        "full" | "f32" => Ok(StorageMode::Full),
        "sq8" | "int8" => Ok(StorageMode::SQ8),
        "binary" | "bit" => Ok(StorageMode::Binary),
        _ => Err(PyValueError::new_err(format!(
            "Invalid storage_mode '{}'. Use 'full', 'sq8', or 'binary'",
            mode
        ))),
    }
}

/// Convert a Python object to a serde_json::Value.
pub fn python_to_json(py: Python<'_>, obj: &PyObject) -> Option<serde_json::Value> {
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

/// Helper to convert a value to PyObject using IntoPyObject trait.
/// This replaces the deprecated `into_py` method.
#[inline]
pub fn to_pyobject<'py, T>(py: Python<'py>, value: T) -> PyObject
where
    T: IntoPyObjectExt<'py>,
{
    value.into_py_any(py).expect("infallible conversion")
}

/// Convert a serde_json::Value to a Python object.
pub fn json_to_python(py: Python<'_>, value: &serde_json::Value) -> PyObject {
    match value {
        serde_json::Value::Null => py.None(),
        serde_json::Value::Bool(b) => to_pyobject(py, *b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                to_pyobject(py, i)
            } else if let Some(f) = n.as_f64() {
                to_pyobject(py, f)
            } else {
                py.None()
            }
        }
        serde_json::Value::String(s) => to_pyobject(py, s.as_str()),
        serde_json::Value::Array(arr) => {
            let list: Vec<PyObject> = arr.iter().map(|v| json_to_python(py, v)).collect();
            to_pyobject(py, list)
        }
        serde_json::Value::Object(map) => {
            let dict: HashMap<String, PyObject> = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_python(py, v)))
                .collect();
            to_pyobject(py, dict)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metric_cosine() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_metric("cosine").unwrap(),
                DistanceMetric::Cosine
            ));
            assert!(matches!(
                parse_metric("COSINE").unwrap(),
                DistanceMetric::Cosine
            ));
        });
    }

    #[test]
    fn test_parse_metric_euclidean() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_metric("euclidean").unwrap(),
                DistanceMetric::Euclidean
            ));
            assert!(matches!(
                parse_metric("l2").unwrap(),
                DistanceMetric::Euclidean
            ));
        });
    }

    #[test]
    fn test_parse_metric_dot() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_metric("dot").unwrap(),
                DistanceMetric::DotProduct
            ));
            assert!(matches!(
                parse_metric("dotproduct").unwrap(),
                DistanceMetric::DotProduct
            ));
            assert!(matches!(
                parse_metric("ip").unwrap(),
                DistanceMetric::DotProduct
            ));
        });
    }

    #[test]
    fn test_parse_metric_hamming() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_metric("hamming").unwrap(),
                DistanceMetric::Hamming
            ));
        });
    }

    #[test]
    fn test_parse_metric_jaccard() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_metric("jaccard").unwrap(),
                DistanceMetric::Jaccard
            ));
        });
    }

    #[test]
    fn test_parse_metric_invalid() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(parse_metric("invalid").is_err());
        });
    }

    #[test]
    fn test_parse_storage_mode_full() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_storage_mode("full").unwrap(),
                StorageMode::Full
            ));
            assert!(matches!(
                parse_storage_mode("f32").unwrap(),
                StorageMode::Full
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_sq8() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_storage_mode("sq8").unwrap(),
                StorageMode::SQ8
            ));
            assert!(matches!(
                parse_storage_mode("int8").unwrap(),
                StorageMode::SQ8
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_binary() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(matches!(
                parse_storage_mode("binary").unwrap(),
                StorageMode::Binary
            ));
            assert!(matches!(
                parse_storage_mode("bit").unwrap(),
                StorageMode::Binary
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_invalid() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|_py| {
            assert!(parse_storage_mode("invalid").is_err());
        });
    }
}
