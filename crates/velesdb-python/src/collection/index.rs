//! Index management methods for Collection (property, range).

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::utils::to_pyobject;

use super::Collection;

#[pymethods]
impl Collection {
    /// Create a property index for O(1) equality lookups.
    #[pyo3(signature = (label, property))]
    fn create_property_index(&self, label: &str, property: &str) -> PyResult<()> {
        self.inner
            .create_property_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create property index: {e}")))
    }

    /// Create a range index for O(log n) range queries.
    #[pyo3(signature = (label, property))]
    fn create_range_index(&self, label: &str, property: &str) -> PyResult<()> {
        self.inner
            .create_range_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create range index: {e}")))
    }

    /// Check if a property index exists.
    #[pyo3(signature = (label, property))]
    fn has_property_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_property_index(label, property)
    }

    /// Check if a range index exists.
    #[pyo3(signature = (label, property))]
    fn has_range_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_range_index(label, property)
    }

    /// List all indexes on this collection.
    ///
    /// Returns:
    ///     List of dicts with keys: label, property, index_type, cardinality, memory_bytes
    fn list_indexes(&self) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let indexes = self.inner.list_indexes();
            let py_indexes: Vec<HashMap<String, PyObject>> = indexes
                .into_iter()
                .map(|idx| {
                    let mut result = HashMap::new();
                    result.insert("label".to_string(), to_pyobject(py, idx.label));
                    result.insert("property".to_string(), to_pyobject(py, idx.property));
                    result.insert("index_type".to_string(), to_pyobject(py, idx.index_type));
                    result.insert("cardinality".to_string(), to_pyobject(py, idx.cardinality));
                    result.insert(
                        "memory_bytes".to_string(),
                        to_pyobject(py, idx.memory_bytes),
                    );
                    result
                })
                .collect();
            Ok(py_indexes)
        })
    }

    /// Drop an index (either property or range).
    ///
    /// Args:
    ///     label: Node label
    ///     property: Property name
    ///
    /// Returns:
    ///     True if an index was dropped, False if no index existed
    #[pyo3(signature = (label, property))]
    fn drop_index(&self, label: &str, property: &str) -> PyResult<bool> {
        self.inner
            .drop_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to drop index: {e}")))
    }
}
