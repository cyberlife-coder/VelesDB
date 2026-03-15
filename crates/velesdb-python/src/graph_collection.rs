//! Persistent `GraphCollection` bindings for VelesDB Python.
//!
//! Wraps `velesdb_core::GraphCollection` (disk-backed, persistent graph)
//! as a `PyGraphCollection` pyclass.  Follows the same patterns as
//! `crate::collection::Collection` (error handling, dict conversion).

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::collection_helpers::search_result_to_dict;
use crate::graph::{edge_to_dict, traversal_to_dict};
use crate::utils::{extract_vector, json_to_python, python_to_json};
use velesdb_core::collection::graph::TraversalConfig;
use velesdb_core::{GraphCollection, GraphSchema};

// ---------------------------------------------------------------------------
// PyGraphSchema
// ---------------------------------------------------------------------------

/// Schema configuration for a graph collection.
///
/// Controls whether the graph enforces strict node/edge types or accepts
/// any type (schemaless).
///
/// Example:
///     >>> schema = PyGraphSchema.schemaless()
///     >>> schema = PyGraphSchema.strict()
#[pyclass(name = "GraphSchema")]
#[derive(Clone)]
pub struct PyGraphSchema {
    inner: GraphSchema,
}

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

    fn __repr__(&self) -> String {
        if self.inner.is_schemaless() {
            "GraphSchema(schemaless=True)".to_string()
        } else {
            "GraphSchema(schemaless=False)".to_string()
        }
    }
}

impl PyGraphSchema {
    /// Returns a reference to the inner `GraphSchema`.
    pub fn inner(&self) -> &GraphSchema {
        &self.inner
    }
}

// ---------------------------------------------------------------------------
// PyGraphCollection
// ---------------------------------------------------------------------------

/// A persistent, disk-backed graph collection.
///
/// Wraps the core `GraphCollection` which stores typed relationships
/// between nodes, with optional node embeddings for vector search.
///
/// Example:
///     >>> db = velesdb.Database("./data")
///     >>> graph = db.create_graph_collection("knowledge")
///     >>> graph.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
///     >>> edges = graph.get_edges()
#[pyclass(name = "GraphCollection")]
pub struct PyGraphCollection {
    inner: GraphCollection,
    name: String,
}

impl PyGraphCollection {
    /// Creates a new `PyGraphCollection` wrapper.
    pub fn new(inner: GraphCollection, name: String) -> Self {
        Self { inner, name }
    }
}

#[pymethods]
impl PyGraphCollection {
    /// The collection name.
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// Returns the graph schema for this collection.
    ///
    /// Returns:
    ///     GraphSchema: The schema configuration
    #[getter]
    fn schema(&self) -> PyGraphSchema {
        PyGraphSchema {
            inner: self.inner.schema(),
        }
    }

    /// Whether this collection has node embeddings enabled.
    ///
    /// Returns:
    ///     bool: True if vector search is available
    #[getter]
    fn has_embeddings(&self) -> bool {
        self.inner.has_embeddings()
    }

    /// Add an edge between two nodes.
    ///
    /// Args:
    ///     edge: Dict with keys: id (int), source (int), target (int),
    ///           label (str), properties (dict, optional)
    ///
    /// Example:
    ///     >>> graph.add_edge({
    ///     ...     "id": 1, "source": 10, "target": 20,
    ///     ...     "label": "KNOWS", "properties": {"since": 2020}
    ///     ... })
    #[pyo3(signature = (edge))]
    fn add_edge(&self, edge: HashMap<String, PyObject>) -> PyResult<()> {
        Python::with_gil(|py| {
            let graph_edge = crate::graph::dict_to_edge(py, &edge)?;
            self.inner
                .add_edge(graph_edge)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to add edge: {e}")))
        })
    }

    /// Get edges, optionally filtered by label.
    ///
    /// Args:
    ///     label: Optional relationship type filter (e.g. "KNOWS")
    ///
    /// Returns:
    ///     List of edge dicts with keys: id, source, target, label, properties
    ///
    /// Example:
    ///     >>> all_edges = graph.get_edges()
    ///     >>> knows_edges = graph.get_edges(label="KNOWS")
    #[pyo3(signature = (label=None))]
    fn get_edges(&self, label: Option<&str>) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let edges = self.inner.get_edges(label);
            Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
        })
    }

    /// Get outgoing edges from a node.
    ///
    /// Args:
    ///     node_id: The source node ID
    ///
    /// Returns:
    ///     List of edge dicts
    #[pyo3(signature = (node_id))]
    fn get_outgoing(&self, node_id: u64) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let edges = self.inner.get_outgoing(node_id);
            Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
        })
    }

    /// Get incoming edges to a node.
    ///
    /// Args:
    ///     node_id: The target node ID
    ///
    /// Returns:
    ///     List of edge dicts
    #[pyo3(signature = (node_id))]
    fn get_incoming(&self, node_id: u64) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let edges = self.inner.get_incoming(node_id);
            Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
        })
    }

    /// Get the in-degree and out-degree of a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///
    /// Returns:
    ///     Tuple of (in_degree, out_degree)
    #[pyo3(signature = (node_id))]
    fn node_degree(&self, node_id: u64) -> (usize, usize) {
        self.inner.node_degree(node_id)
    }

    /// Store payload (properties) for a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///     payload: Dict of properties to store
    ///
    /// Example:
    ///     >>> graph.store_node_payload(10, {"name": "Alice", "age": 30})
    #[pyo3(signature = (node_id, payload))]
    fn store_node_payload(&self, py: Python<'_>, node_id: u64, payload: PyObject) -> PyResult<()> {
        let value = python_to_json(py, &payload).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Failed to convert payload to JSON")
        })?;
        self.inner
            .store_node_payload(node_id, &value)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to store payload: {e}")))
    }

    /// Retrieve payload (properties) for a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///
    /// Returns:
    ///     Dict of properties, or None if no payload is stored
    #[pyo3(signature = (node_id))]
    fn get_node_payload(&self, py: Python<'_>, node_id: u64) -> PyResult<Option<PyObject>> {
        let value = self
            .inner
            .get_node_payload(node_id)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to get payload: {e}")))?;
        Ok(value.map(|v| json_to_python(py, &v)))
    }

    /// Get all node IDs that have a stored payload.
    ///
    /// Returns:
    ///     List of node IDs
    fn all_node_ids(&self) -> Vec<u64> {
        self.inner.all_node_ids()
    }

    /// Perform BFS traversal from a source node.
    ///
    /// Args:
    ///     source_id: Starting node ID
    ///     max_depth: Maximum traversal depth (default: 3)
    ///     limit: Maximum results to return (default: 100)
    ///     rel_types: Optional list of relationship types to follow
    ///
    /// Returns:
    ///     List of traversal result dicts with keys: target_id, path, depth
    #[pyo3(signature = (source_id, max_depth=None, limit=None, rel_types=None))]
    fn traverse_bfs(
        &self,
        source_id: u64,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        let config = build_traversal_config(max_depth, limit, rel_types);
        Python::with_gil(|py| {
            let results = self.inner.traverse_bfs(source_id, &config);
            Ok(results.iter().map(|r| traversal_to_dict(py, r)).collect())
        })
    }

    /// Perform DFS traversal from a source node.
    ///
    /// Args:
    ///     source_id: Starting node ID
    ///     max_depth: Maximum traversal depth (default: 3)
    ///     limit: Maximum results to return (default: 100)
    ///     rel_types: Optional list of relationship types to follow
    ///
    /// Returns:
    ///     List of traversal result dicts with keys: target_id, path, depth
    #[pyo3(signature = (source_id, max_depth=None, limit=None, rel_types=None))]
    fn traverse_dfs(
        &self,
        source_id: u64,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        let config = build_traversal_config(max_depth, limit, rel_types);
        Python::with_gil(|py| {
            let results = self.inner.traverse_dfs(source_id, &config);
            Ok(results.iter().map(|r| traversal_to_dict(py, r)).collect())
        })
    }

    /// Search for similar nodes by embedding vector.
    ///
    /// Only available when the collection was created with a dimension
    /// (i.e. ``has_embeddings`` is True).
    ///
    /// Args:
    ///     query: Query vector (list or numpy array)
    ///     k: Number of results to return (default: 10)
    ///
    /// Returns:
    ///     List of result dicts with keys: id, score, payload
    ///
    /// Raises:
    ///     RuntimeError: If the collection has no embeddings
    #[pyo3(signature = (query, k=None))]
    fn search_by_embedding(
        &self,
        py: Python<'_>,
        query: PyObject,
        k: Option<usize>,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        let vec = extract_vector(py, &query)?;
        let top_k = k.unwrap_or(10);
        let results = self
            .inner
            .search_by_embedding(&vec, top_k)
            .map_err(|e| PyRuntimeError::new_err(format!("Search failed: {e}")))?;
        Ok(results
            .iter()
            .map(|r| search_result_to_dict(py, r))
            .collect())
    }

    /// Flush all graph state to disk.
    ///
    /// Ensures edges, payloads, and indexes are persisted.
    fn flush(&self) -> PyResult<()> {
        self.inner
            .flush()
            .map_err(|e| PyRuntimeError::new_err(format!("Flush failed: {e}")))
    }

    /// Returns the total number of edges in the graph.
    ///
    /// Returns:
    ///     int: Edge count
    fn edge_count(&self) -> usize {
        self.inner.get_edges(None).len()
    }

    fn __repr__(&self) -> String {
        format!(
            "GraphCollection(name='{}', has_embeddings={})",
            self.name,
            self.inner.has_embeddings(),
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `TraversalConfig` from optional Python parameters.
fn build_traversal_config(
    max_depth: Option<u32>,
    limit: Option<usize>,
    rel_types: Option<Vec<String>>,
) -> TraversalConfig {
    TraversalConfig {
        min_depth: 1,
        max_depth: max_depth.unwrap_or(3),
        limit: limit.unwrap_or(100),
        rel_types: rel_types.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_traversal_config_defaults() {
        let config = build_traversal_config(None, None, None);
        assert_eq!(config.min_depth, 1);
        assert_eq!(config.max_depth, 3);
        assert_eq!(config.limit, 100);
        assert!(config.rel_types.is_empty());
    }

    #[test]
    fn test_build_traversal_config_custom() {
        let config = build_traversal_config(Some(5), Some(50), Some(vec!["KNOWS".to_string()]));
        assert_eq!(config.max_depth, 5);
        assert_eq!(config.limit, 50);
        assert_eq!(config.rel_types, vec!["KNOWS"]);
    }

    #[test]
    fn test_py_graph_schema_schemaless() {
        let schema = PyGraphSchema::schemaless();
        assert!(schema.inner.is_schemaless());
    }

    #[test]
    fn test_py_graph_schema_strict() {
        let schema = PyGraphSchema::strict();
        assert!(!schema.inner.is_schemaless());
    }
}
