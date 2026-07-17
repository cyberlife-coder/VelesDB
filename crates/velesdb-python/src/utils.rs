//! Utility functions for Python-Rust type conversions.
//!
//! This module provides helper functions for converting between Python and Rust types,
//! particularly for JSON serialization and distance metric/storage mode parsing.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::IntoPyObjectExt;
use std::collections::HashMap;
use velesdb_core::{DistanceMetric, StorageMode};

/// Extract an optional typed value for `key` from `dict`, returning `None` when
/// the key is absent or holds Python `None` — so `{}`, a missing key, and an
/// explicit `None` all read as "unset". Shared by every binding that takes an
/// options dict (collection config, `recall_fused` fusion options).
///
/// # Errors
///
/// Propagates any error from extracting the value as `T` (e.g. a wrong type).
pub fn opt_field<'py, T: FromPyObjectOwned<'py>>(
    dict: &Bound<'py, PyDict>,
    key: &str,
) -> PyResult<Option<T>> {
    match dict.get_item(key)? {
        Some(v) if !v.is_none() => Ok(Some(v.extract().map_err(Into::into)?)),
        _ => Ok(None),
    }
}

/// Rejects vectors that contain non-finite values (NaN or Infinity).
///
/// NaN propagates silently through distance computations and corrupts
/// search results. Infinity corrupts distance calculations by producing
/// infinite or NaN distances. This check is applied to every vector
/// entering the engine from Python.
///
/// # Errors
///
/// Returns `PyValueError` when any component is NaN or Infinity.
#[inline]
pub fn reject_nan_vector(vector: &[f32]) -> PyResult<()> {
    if vector.iter().any(|v| !v.is_finite()) {
        return Err(PyValueError::new_err(
            "vector contains non-finite values (NaN or Infinity)",
        ));
    }
    Ok(())
}

/// Extracts a vector from a Py<PyAny>, supporting both Python lists and NumPy arrays.
///
/// After extraction the vector is validated: NaN values are rejected with a
/// `PyValueError`.
///
/// # Arguments
/// * `py` - Python GIL token
/// * `obj` - The Python object (list or numpy.ndarray)
///
/// # Returns
/// A `Vec<f32>` containing the validated vector data
///
/// # Errors
/// Returns an error if the object is neither a list nor a numpy array,
/// or if the extracted vector contains NaN values.
pub fn extract_vector(py: Python<'_>, obj: &Py<PyAny>) -> PyResult<Vec<f32>> {
    // Try numpy array first (most common in ML workflows)
    if let Ok(array) = obj.extract::<numpy::PyReadonlyArray1<f32>>(py) {
        let vec = array.as_slice()?.to_vec();
        reject_nan_vector(&vec)?;
        return Ok(vec);
    }

    // Try numpy float64 array and convert
    if let Ok(array) = obj.extract::<numpy::PyReadonlyArray1<f64>>(py) {
        // Reason: intentional f64->f32 narrowing for numpy float64 arrays
        #[allow(clippy::cast_possible_truncation)]
        let vec: Vec<f32> = array.as_slice()?.iter().map(|&x| x as f32).collect();
        reject_nan_vector(&vec)?;
        return Ok(vec);
    }

    // Fall back to Python list
    if let Ok(list) = obj.extract::<Vec<f32>>(py) {
        reject_nan_vector(&list)?;
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
        .map_err(PyValueError::new_err)
}

/// Parse a storage mode string into a `StorageMode` enum.
///
/// Delegates to [`StorageMode::from_str`] (single source of truth in `velesdb-core`).
pub fn parse_storage_mode(mode: &str) -> PyResult<StorageMode> {
    mode.parse::<StorageMode>().map_err(PyValueError::new_err)
}

/// Convert a Python object to a `serde_json::Value`.
///
/// Returns `Err` for unsupported Python types (datetime, UUID, bytes, custom objects)
/// instead of silently dropping them.
///
/// # Errors
///
/// Returns `PyValueError` if the Python object type is not JSON-serializable.
pub fn python_to_json(py: Python<'_>, obj: &Py<PyAny>) -> PyResult<serde_json::Value> {
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
    // A Python int outside i64's range (up to u64::MAX) still round-trips
    // exactly as a JSON number — needed for u64 ids (e.g. the context
    // compiler's FNV-1a `stable_id`, which is uniform over u64 and so
    // exceeds i64::MAX roughly half the time). Falling through to f64 here
    // would silently truncate precision instead of erroring or round-tripping.
    if let Ok(u) = obj.extract::<u64>(py) {
        return Ok(serde_json::Value::Number(u.into()));
    }
    // Any remaining Python int is outside [i64::MIN, u64::MAX] (unbounded
    // precision on the Python side). Reject it explicitly: letting it fall
    // through to the f64 branch below would silently round it — exactly the
    // lossy degradation the u64 branch above exists to prevent.
    if obj.bind(py).is_instance_of::<pyo3::types::PyInt>() {
        return Err(PyValueError::new_err(format!(
            "Integer out of the supported range [{}, {}] (i64::MIN to u64::MAX); \
             larger values would silently lose precision",
            i64::MIN,
            u64::MAX
        )));
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
    if let Ok(list) = obj.extract::<Vec<Py<PyAny>>>(py) {
        let arr: Vec<serde_json::Value> = list
            .iter()
            .map(|item| python_to_json(py, item))
            .collect::<PyResult<_>>()?;
        return Ok(serde_json::Value::Array(arr));
    }
    if let Ok(dict) = obj.extract::<HashMap<String, Py<PyAny>>>(py) {
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

/// Helper to convert a value to `Py<PyAny>` using the `IntoPyObject` trait.
///
/// Returns `py.None()` on conversion failure instead of panicking,
/// which preserves the existing `-> Py<PyAny>` signature for 18 callers.
/// Failures are logged to help diagnose unexpected `None` values in results.
#[inline]
pub fn to_pyobject<'py, T>(py: Python<'py>, value: T) -> Py<PyAny>
where
    T: IntoPyObjectExt<'py>,
{
    value.into_py_any(py).unwrap_or_else(|e| {
        eprintln!("[velesdb] to_pyobject conversion failed, returning None: {e}");
        py.None()
    })
}

/// Convert a `serde_json::Value` to a Python object.
///
/// Builds `PyDict`/`PyList` directly instead of going through `HashMap`/`Vec`
/// intermediaries to avoid unnecessary allocations.
pub fn json_to_python(py: Python<'_>, value: &serde_json::Value) -> Py<PyAny> {
    use pyo3::types::{PyDict, PyList};

    match value {
        serde_json::Value::Null => py.None(),
        serde_json::Value::Bool(b) => to_pyobject(py, *b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                to_pyobject(py, i)
            } else if let Some(u) = n.as_u64() {
                // A u64 that does not fit i64 (e.g. a `stable_id` above
                // i64::MAX) — convert directly instead of falling through to
                // f64, which would silently lose precision.
                to_pyobject(py, u)
            } else if let Some(f) = n.as_f64() {
                to_pyobject(py, f)
            } else {
                py.None()
            }
        }
        serde_json::Value::String(s) => to_pyobject(py, s.as_str()),
        serde_json::Value::Array(arr) => {
            let items: Vec<Py<PyAny>> = arr.iter().map(|v| json_to_python(py, v)).collect();
            // PyList::new is infallible for Vec<Py<PyAny>> items.
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
    fn test_reject_nan_vector_clean() {
        assert!(reject_nan_vector(&[1.0, 2.0, 3.0]).is_ok());
    }

    #[test]
    fn test_reject_nan_vector_contains_nan() {
        let err = reject_nan_vector(&[1.0, f32::NAN, 3.0]).unwrap_err();
        assert!(err.to_string().contains("non-finite"));
    }

    #[test]
    fn test_reject_nan_vector_empty() {
        assert!(reject_nan_vector(&[]).is_ok());
    }

    #[test]
    fn test_reject_nan_vector_positive_infinity() {
        let err = reject_nan_vector(&[1.0, f32::INFINITY, 3.0]).unwrap_err();
        assert!(err.to_string().contains("non-finite"));
    }

    #[test]
    fn test_reject_nan_vector_negative_infinity() {
        let err = reject_nan_vector(&[1.0, f32::NEG_INFINITY, 3.0]).unwrap_err();
        assert!(err.to_string().contains("non-finite"));
    }

    #[test]
    fn test_extract_vector_rejects_nan_list() {
        pyo3::Python::initialize();
        Python::attach(|py| {
            let list = vec![1.0_f32, f32::NAN, 3.0];
            let obj: Py<PyAny> = list
                .into_pyobject(py)
                .expect("test: convert Vec<f32> to Python list")
                .into();
            let err = extract_vector(py, &obj).unwrap_err();
            assert!(err.to_string().contains("non-finite"));
        });
    }

    #[test]
    fn test_parse_metric_cosine() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_metric("cosine").expect("test: 'cosine' is a valid metric"),
                DistanceMetric::Cosine
            ));
            assert!(matches!(
                parse_metric("COSINE")
                    .expect("test: 'COSINE' is a valid metric (case-insensitive)"),
                DistanceMetric::Cosine
            ));
        });
    }

    #[test]
    fn test_parse_metric_euclidean() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_metric("euclidean").expect("test: 'euclidean' is a valid metric"),
                DistanceMetric::Euclidean
            ));
            assert!(matches!(
                parse_metric("l2").expect("test: 'l2' is an alias for euclidean"),
                DistanceMetric::Euclidean
            ));
        });
    }

    #[test]
    fn test_parse_metric_dot() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_metric("dot").expect("test: 'dot' is a valid metric"),
                DistanceMetric::DotProduct
            ));
            assert!(matches!(
                parse_metric("dotproduct").expect("test: 'dotproduct' is an alias for dot"),
                DistanceMetric::DotProduct
            ));
            assert!(matches!(
                parse_metric("ip").expect("test: 'ip' (inner product) is an alias for dot"),
                DistanceMetric::DotProduct
            ));
        });
    }

    #[test]
    fn test_parse_metric_hamming() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_metric("hamming").expect("test: 'hamming' is a valid metric"),
                DistanceMetric::Hamming
            ));
        });
    }

    #[test]
    fn test_parse_metric_jaccard() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_metric("jaccard").expect("test: 'jaccard' is a valid metric"),
                DistanceMetric::Jaccard
            ));
        });
    }

    #[test]
    fn test_parse_metric_invalid() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(parse_metric("invalid").is_err());
        });
    }

    #[test]
    fn test_parse_storage_mode_full() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_storage_mode("full").expect("test: 'full' is a valid storage mode"),
                StorageMode::Full
            ));
            assert!(matches!(
                parse_storage_mode("f32").expect("test: 'f32' is an alias for full"),
                StorageMode::Full
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_sq8() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_storage_mode("sq8").expect("test: 'sq8' is a valid storage mode"),
                StorageMode::SQ8
            ));
            assert!(matches!(
                parse_storage_mode("int8").expect("test: 'int8' is an alias for sq8"),
                StorageMode::SQ8
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_binary() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_storage_mode("binary").expect("test: 'binary' is a valid storage mode"),
                StorageMode::Binary
            ));
            assert!(matches!(
                parse_storage_mode("bit").expect("test: 'bit' is an alias for binary"),
                StorageMode::Binary
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_pq() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_storage_mode("pq").expect("test: 'pq' is a valid storage mode"),
                StorageMode::ProductQuantization
            ));
            assert!(matches!(
                parse_storage_mode("product_quantization")
                    .expect("test: 'product_quantization' is an alias for pq"),
                StorageMode::ProductQuantization
            ));
            // Case-insensitive (delegates to core `StorageMode::from_str`).
            assert!(matches!(
                parse_storage_mode("PQ").expect("test: 'PQ' is case-insensitive alias for pq"),
                StorageMode::ProductQuantization
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_rabitq() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(matches!(
                parse_storage_mode("rabitq").expect("test: 'rabitq' is a valid storage mode"),
                StorageMode::RaBitQ
            ));
            // Case-insensitive (delegates to core `StorageMode::from_str`).
            assert!(matches!(
                parse_storage_mode("RaBitQ")
                    .expect("test: 'RaBitQ' is case-insensitive alias for rabitq"),
                StorageMode::RaBitQ
            ));
            assert!(matches!(
                parse_storage_mode("RABITQ")
                    .expect("test: 'RABITQ' is case-insensitive alias for rabitq"),
                StorageMode::RaBitQ
            ));
        });
    }

    #[test]
    fn test_parse_storage_mode_invalid() {
        pyo3::Python::initialize();
        Python::attach(|_py| {
            assert!(parse_storage_mode("invalid").is_err());
        });
    }
}
