//! Graph and index management methods for Collection (extracted from collection.rs).
//!
//! Contains: Index management (EPIC-009) and Graph operations (EPIC-015 US-001).

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::collection::Collection;
use crate::utils::{python_to_json, to_pyobject};
use velesdb_core::collection::graph::GraphEdge;

#[pymethods]
impl Collection {
    // ========================================================================
    // Index Management (EPIC-009 propagation)
    // ========================================================================

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
    ///
    /// Example:
    ///     >>> indexes = collection.list_indexes()
    ///     >>> for idx in indexes:
    ///     ...     print(f"{idx['label']}.{idx['property']} ({idx['index_type']})")
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
    ///
    /// Example:
    ///     >>> dropped = collection.drop_index("Person", "email")
    #[pyo3(signature = (label, property))]
    fn drop_index(&self, label: &str, property: &str) -> PyResult<bool> {
        self.inner
            .drop_index(label, property)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to drop index: {e}")))
    }

    // ========================================================================
    // Graph Operations (EPIC-015 US-001)
    // ========================================================================

    /// Add an edge to the collection's knowledge graph.
    ///
    /// Args:
    ///     id: Edge ID (must be unique)
    ///     source: Source node ID
    ///     target: Target node ID
    ///     label: Relationship type/label
    ///     metadata: Optional edge properties (dict)
    ///
    /// Example:
    ///     >>> collection.add_edge(1, 100, 200, "RELATED_TO", {"weight": 0.95})
    #[pyo3(signature = (id, source, target, label, metadata = None))]
    fn add_edge(
        &self,
        id: u64,
        source: u64,
        target: u64,
        label: &str,
        metadata: Option<HashMap<String, PyObject>>,
    ) -> PyResult<()> {
        Python::with_gil(|py| {
            let mut edge = GraphEdge::new(id, source, target, label)
                .map_err(|e| PyValueError::new_err(format!("Invalid edge: {e}")))?;

            // Add metadata if provided
            if let Some(meta) = metadata {
                let mut properties = std::collections::HashMap::new();
                for (key, value) in meta {
                    if let Some(json_val) = python_to_json(py, &value) {
                        properties.insert(key, json_val);
                    }
                }
                edge = edge.with_properties(properties);
            }

            self.inner
                .add_edge(edge)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to add edge: {e}")))
        })
    }

    /// Get all edges from the collection's knowledge graph.
    ///
    /// Returns:
    ///     List of edge dicts with id, source, target, label, and metadata keys
    ///
    /// Example:
    ///     >>> edges = collection.get_edges()
    ///     >>> for edge in edges:
    ///     ...     print(f"Edge {edge['id']}: {edge['source']} -> {edge['target']} ({edge['label']})")
    fn get_edges(&self) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let edges = self.inner.get_all_edges();
            let py_edges = edges
                .into_iter()
                .map(|edge| crate::graph::edge_to_dict(py, &edge))
                .collect();
            Ok(py_edges)
        })
    }

    /// Get edges filtered by label (relationship type).
    ///
    /// Args:
    ///     label: Relationship type to filter by
    ///
    /// Returns:
    ///     List of edge dicts matching the label
    ///
    /// Example:
    ///     >>> related_edges = collection.get_edges_by_label("RELATED_TO")
    fn get_edges_by_label(&self, label: &str) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let edges = self.inner.get_edges_by_label(label);
            let py_edges = edges
                .into_iter()
                .map(|edge| crate::graph::edge_to_dict(py, &edge))
                .collect();
            Ok(py_edges)
        })
    }

    /// Traverse the graph from a source node using BFS or DFS.
    ///
    /// Args:
    ///     source: Starting node ID
    ///     max_depth: Maximum traversal depth (default: 2)
    ///     strategy: Traversal strategy, either "bfs" or "dfs" (default: "bfs")
    ///     limit: Maximum number of results (default: 100)
    ///
    /// Returns:
    ///     List of traversal result dicts with target_id, depth, and path keys
    ///
    /// Example:
    ///     >>> results = collection.traverse(100, max_depth=3, strategy="bfs")
    ///     >>> for result in results:
    ///     ...     print(f"Found node {result['target_id']} at depth {result['depth']}")
    #[pyo3(signature = (source, max_depth = 2, strategy = "bfs", limit = 100))]
    fn traverse(
        &self,
        source: u64,
        max_depth: u32,
        strategy: &str,
        limit: usize,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            // Dispatch to appropriate traversal method
            let results = match strategy {
                "bfs" => self.inner.traverse_bfs(source, max_depth, None, limit),
                "dfs" => self.inner.traverse_dfs(source, max_depth, None, limit),
                _ => return Err(PyValueError::new_err("strategy must be 'bfs' or 'dfs'")),
            };

            let results =
                results.map_err(|e| PyRuntimeError::new_err(format!("Traversal failed: {e}")))?;

            let py_results = results
                .into_iter()
                .map(|result| {
                    let mut dict = HashMap::new();
                    dict.insert("target_id".to_string(), to_pyobject(py, result.target_id));
                    dict.insert("depth".to_string(), to_pyobject(py, result.depth));
                    dict.insert("path".to_string(), to_pyobject(py, result.path));
                    dict
                })
                .collect();

            Ok(py_results)
        })
    }

    /// Get the in-degree and out-degree of a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///
    /// Returns:
    ///     Dict with node_id, in_degree, out_degree, and total_degree keys
    ///
    /// Example:
    ///     >>> degree = collection.get_node_degree(100)
    ///     >>> print(f"Node 100 has {degree['total_degree']} connections")
    fn get_node_degree(&self, node_id: u64) -> PyResult<HashMap<String, PyObject>> {
        Python::with_gil(|py| {
            let (in_degree, out_degree) = self.inner.get_node_degree(node_id);
            let mut degree_dict = HashMap::new();
            degree_dict.insert("node_id".to_string(), to_pyobject(py, node_id));
            degree_dict.insert("in_degree".to_string(), to_pyobject(py, in_degree));
            degree_dict.insert("out_degree".to_string(), to_pyobject(py, out_degree));
            degree_dict.insert(
                "total_degree".to_string(),
                to_pyobject(py, in_degree + out_degree),
            );
            Ok(degree_dict)
        })
    }
}
