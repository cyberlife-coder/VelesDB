//! Index management methods for Collection (property, range).

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};

use crate::collection_helpers::core_err;

use super::Collection;

#[pymethods]
impl Collection {
    /// Create a property index for O(1) equality lookups.
    #[pyo3(signature = (label, property))]
    fn create_property_index(&self, py: Python<'_>, label: &str, property: &str) -> PyResult<()> {
        let label_owned = label.to_string();
        let property_owned = property.to_string();
        py.detach(|| {
            self.inner
                .create_property_index(&label_owned, &property_owned)
                .map_err(core_err)
        })
    }

    /// Create a range index for O(log n) range queries.
    #[pyo3(signature = (label, property))]
    fn create_range_index(&self, py: Python<'_>, label: &str, property: &str) -> PyResult<()> {
        let label_owned = label.to_string();
        let property_owned = property.to_string();
        py.detach(|| {
            self.inner
                .create_range_index(&label_owned, &property_owned)
                .map_err(core_err)
        })
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
    fn list_indexes(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let indexes = self.inner.list_indexes();
        let py_indexes: Vec<Py<PyAny>> = indexes
            .into_iter()
            .map(|idx| {
                let dict = PyDict::new(py);
                let _ = dict.set_item(PyString::intern(py, "label"), idx.label);
                let _ = dict.set_item(PyString::intern(py, "property"), idx.property);
                let _ = dict.set_item(PyString::intern(py, "index_type"), idx.index_type);
                let _ = dict.set_item(PyString::intern(py, "cardinality"), idx.cardinality);
                let _ = dict.set_item(PyString::intern(py, "memory_bytes"), idx.memory_bytes);
                dict.into_any().unbind()
            })
            .collect();
        Ok(py_indexes)
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
    fn drop_index(&self, py: Python<'_>, label: &str, property: &str) -> PyResult<bool> {
        let label_owned = label.to_string();
        let property_owned = property.to_string();
        py.detach(|| {
            self.inner
                .drop_index(&label_owned, &property_owned)
                .map_err(core_err)
        })
    }
}
