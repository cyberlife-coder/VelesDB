//! `#[pymethods]` for `PyGraphSchema` — split out of `graph_collection.rs`
//! to stay under the file budget. The pyclass itself and its inherent
//! impl stay in `graph_collection.rs` (the release-snapshot API path).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;

use velesdb_core::{EdgeType, GraphSchema, NodeType, ValueType};

use crate::graph_collection::PyGraphSchema;

#[pymethods]
impl PyGraphSchema {
    /// Create a schemaless graph schema that accepts any node/edge types.
    ///
    /// Returns:
    ///     GraphSchema: A schemaless schema instance
    ///
    /// Example:
    ///     >>> schema = GraphSchema.schemaless()
    #[staticmethod]
    fn schemaless() -> Self {
        Self {
            inner: GraphSchema::schemaless(),
        }
    }

    /// Create a strict graph schema (only predefined types allowed).
    ///
    /// Returns:
    ///     GraphSchema: A strict schema instance
    ///
    /// Example:
    ///     >>> schema = GraphSchema.strict()
    #[staticmethod]
    fn strict() -> Self {
        Self {
            inner: GraphSchema::new(),
        }
    }

    /// Check whether this schema is schemaless.
    ///
    /// Returns:
    ///     bool: True if the schema accepts any types
    #[getter]
    fn is_schemaless(&self) -> bool {
        self.inner.is_schemaless()
    }

    /// Declare a node type on a strict schema (builder — returns a new schema).
    ///
    /// Args:
    ///     name: Node type name (used in the ``_labels`` payload field).
    ///     properties: Optional mapping of property name to value type
    ///         ("string", "integer", "float", "boolean", "vector").
    ///
    /// Returns:
    ///     GraphSchema: A new schema with this node type added
    ///
    /// Example:
    ///     >>> schema = GraphSchema.strict().with_node_type(
    ///     ...     "Person", {"name": "string", "age": "integer"})
    #[pyo3(signature = (name, properties=None))]
    fn with_node_type(
        &self,
        name: &str,
        properties: Option<HashMap<String, String>>,
    ) -> PyResult<Self> {
        let props = parse_property_types(properties)?;
        let inner = self
            .inner
            .clone()
            .with_node_type(NodeType::new(name).with_properties(props));
        Ok(Self { inner })
    }

    /// Declare an edge type on a strict schema (builder — returns a new schema).
    ///
    /// Args:
    ///     name: Edge type (label) name, e.g. "KNOWS".
    ///     from_type: Node type of the source endpoint.
    ///     to_type: Node type of the target endpoint.
    ///     properties: Optional mapping of property name to value type.
    ///
    /// Returns:
    ///     GraphSchema: A new schema with this edge type added
    ///
    /// Example:
    ///     >>> schema = (GraphSchema.strict()
    ///     ...     .with_node_type("Person")
    ///     ...     .with_edge_type("KNOWS", "Person", "Person", {"since": "integer"}))
    #[pyo3(signature = (name, from_type, to_type, properties=None))]
    fn with_edge_type(
        &self,
        name: &str,
        from_type: &str,
        to_type: &str,
        properties: Option<HashMap<String, String>>,
    ) -> PyResult<Self> {
        let props = parse_property_types(properties)?;
        let inner = self
            .inner
            .clone()
            .with_edge_type(EdgeType::new(name, from_type, to_type).with_properties(props));
        Ok(Self { inner })
    }

    /// The declared node types, as a list of
    /// ``{"name": str, "properties": {name: value_type}}`` dicts.
    #[getter]
    fn node_types(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        self.inner
            .node_types()
            .iter()
            .map(|nt| {
                let d = PyDict::new(py);
                d.set_item("name", nt.name())?;
                d.set_item("properties", property_types_to_dict(py, nt.properties())?)?;
                Ok(d.into_any().unbind())
            })
            .collect()
    }

    /// The declared edge types, as a list of
    /// ``{"name": str, "from_type": str, "to_type": str, "properties": {...}}``.
    #[getter]
    fn edge_types(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        self.inner
            .edge_types()
            .iter()
            .map(|et| {
                let d = PyDict::new(py);
                d.set_item("name", et.name())?;
                d.set_item("from_type", et.from_type())?;
                d.set_item("to_type", et.to_type())?;
                d.set_item("properties", property_types_to_dict(py, et.properties())?)?;
                Ok(d.into_any().unbind())
            })
            .collect()
    }

    fn __repr__(&self) -> String {
        if self.inner.is_schemaless() {
            "GraphSchema(schemaless=True)".to_string()
        } else {
            format!(
                "GraphSchema(schemaless=False, node_types={}, edge_types={})",
                self.inner.node_types().len(),
                self.inner.edge_types().len()
            )
        }
    }
}

/// Parse a ``{property: type_name}`` mapping into core `ValueType`s.
fn parse_property_types(
    properties: Option<HashMap<String, String>>,
) -> PyResult<HashMap<String, ValueType>> {
    let mut out = HashMap::new();
    for (key, type_name) in properties.unwrap_or_default() {
        out.insert(key, value_type_from_str(&type_name)?);
    }
    Ok(out)
}

/// Map a value-type name to core's `ValueType` (serde names plus a few aliases).
fn value_type_from_str(name: &str) -> PyResult<ValueType> {
    match name.to_ascii_lowercase().as_str() {
        "string" | "str" | "text" => Ok(ValueType::String),
        "integer" | "int" | "i64" => Ok(ValueType::Integer),
        "float" | "double" | "f64" => Ok(ValueType::Float),
        "boolean" | "bool" => Ok(ValueType::Boolean),
        "vector" | "embedding" => Ok(ValueType::Vector),
        other => Err(PyValueError::new_err(format!(
            "unknown value type '{other}'; expected one of: string, integer, float, boolean, vector"
        ))),
    }
}

/// Render a `ValueType` as its canonical (serde) name.
fn value_type_to_str(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::String => "string",
        ValueType::Integer => "integer",
        ValueType::Float => "float",
        ValueType::Boolean => "boolean",
        ValueType::Vector => "vector",
        _ => "unknown", // ValueType is #[non_exhaustive]
    }
}

/// Build a Python ``{name: type_name}`` dict from a core property map.
fn property_types_to_dict(
    py: Python<'_>,
    properties: &HashMap<String, ValueType>,
) -> PyResult<Py<PyAny>> {
    let d = PyDict::new(py);
    for (key, value_type) in properties {
        d.set_item(key, value_type_to_str(value_type))?;
    }
    Ok(d.into_any().unbind())
}

#[cfg(test)]
#[path = "graph_schema_tests.rs"]
mod tests;
