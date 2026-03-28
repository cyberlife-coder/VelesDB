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
        // Reason: intentional f64->f32 narrowing for numpy float64 arrays
        #[allow(clippy::cast_possible_truncation)]
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

/// Parse a distance metric string into a `DistanceMetric` enum.
///
/// Delegates to [`DistanceMetric::from_str`] to keep alias parsing in one place.
pub fn parse_metric(metric: &str) -> PyResult<DistanceMetric> {
    metric
        .parse::<DistanceMetric>()
        .map_err(|e| PyValueError::new_err(e))
}

/// Parse a storage mode string into a `StorageMode` enum.
///
/// Delegates to [`StorageMode::from_str`] (single source of truth in `velesdb-core`).
pub fn parse_storage_mode(mode: &str) -> PyResult<StorageMode> {
    mode.parse::<StorageMode>()
        .map_err(|e| PyValueError::new_err(e))
}

/// Convert a Python object to a `serde_json::Value`.
///
/// Returns `Err` for unsupported Python types (datetime, UUID, bytes, custom objects)
/// instead of silently dropping them.
///
/// # Errors
///
/// Returns `PyValueError` if the Python object type is not JSON-serializable.
pub fn python_to_json(py: Python<'_>, obj: &PyObject) -> PyResult<serde_json::Value> {
    if let Ok(s) = obj.extract::<String>(py) {
        return Ok(serde_json::Value::String(s));
    }
    // Note: bool MUST be checked before i64 — Python bool is a subclass of int,
    // so extract::<i64>() succeeds on True/False, silently converting them to 1/0.
    if let Ok(b) = obj.extract::<bool>(py) {
        return Ok(serde_json::Value::Bool(b));
    }
    if let Ok(i) = obj.extract::<i64>(py) {
        return Ok(serde_json::Value::Number(i.into()));
    }
    if let Ok(f) = obj.extract::<f64>(py) {
        return serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                PyValueError::new_err("Float value is not JSON-serializable (NaN/Inf)")
            });
    }
    if obj.is_none(py) {
        return Ok(serde_json::Value::Null);
    }
    if let Ok(list) = obj.extract::<Vec<PyObject>>(py) {
        let arr: Vec<serde_json::Value> = list
            .iter()
            .map(|item| python_to_json(py, item))
            .collect::<PyResult<_>>()?;
        return Ok(serde_json::Value::Array(arr));
    }
    if let Ok(dict) = obj.extract::<HashMap<String, PyObject>>(py) {
        let map: serde_json::Map<String, serde_json::Value> = dict
            .into_iter()
            .map(|(k, v)| python_to_json(py, &v).map(|jv| (k, jv)))
            .collect::<PyResult<_>>()?;
        return Ok(serde_json::Value::Object(map));
    }
    let type_name = obj
        .bind(py)
        .get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    Err(PyValueError::new_err(format!(
        "Unsupported payload type '{type_name}'. Supported: str, int, float, bool, None, list, dict"
    )))
}

/// Helper to convert a value to `PyObject` using the `IntoPyObject` trait.
///
/// Returns `py.None()` on conversion failure instead of panicking,
/// which preserves the existing `-> PyObject` signature for 50+ callers.
#[inline]
pub fn to_pyobject<'py, T>(py: Python<'py>, value: T) -> PyObject
where
    T: IntoPyObjectExt<'py>,
{
    value.into_py_any(py).unwrap_or_else(|_| py.None())
}

/// Convert a `serde_json::Value` to a Python object.
///
/// Builds `PyDict`/`PyList` directly instead of going through `HashMap`/`Vec`
/// intermediaries to avoid unnecessary allocations.
pub fn json_to_python(py: Python<'_>, value: &serde_json::Value) -> PyObject {
    use pyo3::types::{PyDict, PyList};

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
            let items: Vec<PyObject> = arr.iter().map(|v| json_to_python(py, v)).collect();
            // PyList::new is infallible for Vec<PyObject> items.
            let list = PyList::new(py, &items).unwrap_or_else(|_| PyList::empty(py));
            list.into_any().unbind()
        }
        serde_json::Value::Object(map) => {
            let dict = PyDict::new(py);
            for (k, v) in map {
                let _ = dict.set_item(k.as_str(), json_to_python(py, v));
            }
            dict.into_any().unbind()
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
