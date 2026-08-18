use super::*;
use pyo3::Python;

/// extract_point_id error message includes the batch index.
#[test]
fn test_extract_point_id_missing_includes_index() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        // Dict with no "id" key — simulates a malformed point at position 4237.
        let empty: HashMap<String, Py<PyAny>> = HashMap::new();
        let err = extract_point_id(py, &empty, 4237).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("4237"),
            "error message must contain the batch index; got: {msg}"
        );
        assert!(
            msg.contains("'id'"),
            "error message must mention the missing field; got: {msg}"
        );
    });
}

/// extract_point_id at index 0 still includes the index in the message.
#[test]
fn test_extract_point_id_missing_at_index_zero() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let empty: HashMap<String, Py<PyAny>> = HashMap::new();
        let err = extract_point_id(py, &empty, 0).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("index 0"),
            "error message must contain 'index 0'; got: {msg}"
        );
    });
}
