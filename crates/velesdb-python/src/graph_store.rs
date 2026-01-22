//! GraphStore bindings for VelesDB Python.
//!
//! Provides PyO3 wrappers for graph operations including:
//! - Edge management (add, get, remove)
//! - Label-based queries (US-030)
//! - BFS streaming traversal (US-032)
//!
//! [EPIC-016/US-030, US-032]

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::graph::{dict_to_edge, edge_to_dict};
use velesdb_core::collection::graph::EdgeStore;

/// Configuration for streaming BFS traversal.
///
/// Example:
///     >>> config = StreamingConfig(max_depth=3, max_visited=10000)
///     >>> config.relationship_types = ["KNOWS", "FOLLOWS"]
#[pyclass]
#[derive(Clone)]
pub struct StreamingConfig {
    /// Maximum traversal depth (default: 3).
    #[pyo3(get, set)]
    pub max_depth: usize,
    /// Maximum nodes to visit (memory bound, default: 10000).
    #[pyo3(get, set)]
    pub max_visited: usize,
    /// Optional filter by relationship types.
    #[pyo3(get, set)]
    pub relationship_types: Option<Vec<String>>,
}

#[pymethods]
impl StreamingConfig {
    #[new]
    #[pyo3(signature = (max_depth=3, max_visited=10000, relationship_types=None))]
    fn new(max_depth: usize, max_visited: usize, relationship_types: Option<Vec<String>>) -> Self {
        Self {
            max_depth,
            max_visited,
            relationship_types,
        }
    }
}

/// Result of a BFS traversal step.
#[pyclass]
#[derive(Clone)]
pub struct TraversalResult {
    /// Current depth in the traversal.
    #[pyo3(get)]
    pub depth: usize,
    /// Source node ID.
    #[pyo3(get)]
    pub source: u64,
    /// Target node ID.
    #[pyo3(get)]
    pub target: u64,
    /// Edge label.
    #[pyo3(get)]
    pub label: String,
    /// Edge ID.
    #[pyo3(get)]
    pub edge_id: u64,
}

#[pymethods]
impl TraversalResult {
    fn __repr__(&self) -> String {
        format!(
            "TraversalResult(depth={}, source={}, target={}, label='{}')",
            self.depth, self.source, self.target, self.label
        )
    }
}

/// In-memory graph store for knowledge graph operations.
///
/// Example:
///     >>> store = GraphStore()
///     >>> store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
///     >>> edges = store.get_edges_by_label("KNOWS")
///     >>> for result in store.traverse_bfs_streaming(100, StreamingConfig()):
///     ...     print(f"Depth {result.depth}: {result.source} -> {result.target}")
#[pyclass]
pub struct GraphStore {
    inner: Arc<std::sync::RwLock<EdgeStore>>,
}

#[pymethods]
impl GraphStore {
    /// Creates a new empty graph store.
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(std::sync::RwLock::new(EdgeStore::new())),
        }
    }

    /// Adds an edge to the graph.
    ///
    /// Args:
    ///     edge: Dict with keys: id (int), source (int), target (int), label (str),
    ///           properties (dict, optional)
    #[pyo3(signature = (edge))]
    fn add_edge(&self, edge: HashMap<String, PyObject>) -> PyResult<()> {
        Python::with_gil(|py| {
            let graph_edge = dict_to_edge(py, &edge)?;
            let mut store = self
                .inner
                .write()
                .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
            store
                .add_edge(graph_edge)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to add edge: {e}")))
        })
    }

    /// Gets all edges with the specified label.
    ///
    /// Args:
    ///     label: The relationship type to filter by (e.g., "KNOWS", "FOLLOWS")
    ///
    /// Returns:
    ///     List of edge dicts with keys: id, source, target, label, properties
    ///
    /// Note:
    ///     Uses internal label index for O(1) lookup per label.
    #[pyo3(signature = (label))]
    fn get_edges_by_label(&self, label: &str) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let store = self
                .inner
                .read()
                .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
            let edges = store.get_edges_by_label(label);
            Ok(edges.into_iter().map(|e| edge_to_dict(py, e)).collect())
        })
    }

    /// Gets outgoing edges from a node.
    #[pyo3(signature = (node_id))]
    fn get_outgoing(&self, node_id: u64) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let store = self
                .inner
                .read()
                .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
            let edges = store.get_outgoing(node_id);
            Ok(edges.into_iter().map(|e| edge_to_dict(py, e)).collect())
        })
    }

    /// Gets incoming edges to a node.
    #[pyo3(signature = (node_id))]
    fn get_incoming(&self, node_id: u64) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let store = self
                .inner
                .read()
                .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
            let edges = store.get_incoming(node_id);
            Ok(edges.into_iter().map(|e| edge_to_dict(py, e)).collect())
        })
    }

    /// Gets outgoing edges filtered by label.
    #[pyo3(signature = (node_id, label))]
    fn get_outgoing_by_label(
        &self,
        node_id: u64,
        label: &str,
    ) -> PyResult<Vec<HashMap<String, PyObject>>> {
        Python::with_gil(|py| {
            let store = self
                .inner
                .read()
                .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
            let edges = store.get_outgoing_by_label(node_id, label);
            Ok(edges.into_iter().map(|e| edge_to_dict(py, e)).collect())
        })
    }

    /// Performs streaming BFS traversal from a start node.
    ///
    /// Args:
    ///     start_node: The node ID to start traversal from
    ///     config: StreamingConfig with max_depth, max_visited, relationship_types
    ///
    /// Returns:
    ///     List of TraversalResult objects (use as iterator for memory efficiency)
    ///
    /// Note:
    ///     Results are bounded by config.max_visited to prevent memory exhaustion.
    ///
    /// Example:
    ///     >>> config = StreamingConfig(max_depth=2, max_visited=100)
    ///     >>> for result in store.traverse_bfs_streaming(100, config):
    ///     ...     print(f"{result.source} -> {result.target}")
    #[pyo3(signature = (start_node, config))]
    fn traverse_bfs_streaming(
        &self,
        start_node: u64,
        config: StreamingConfig,
    ) -> PyResult<Vec<TraversalResult>> {
        let store = self
            .inner
            .read()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;

        let mut results = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut queue: VecDeque<(u64, usize)> = VecDeque::new();

        visited.insert(start_node);
        queue.push_back((start_node, 0));

        let label_filter: Option<HashSet<&str>> = config
            .relationship_types
            .as_ref()
            .map(|types| types.iter().map(String::as_str).collect());

        while let Some((current_node, depth)) = queue.pop_front() {
            if depth >= config.max_depth {
                continue;
            }

            let outgoing = store.get_outgoing(current_node);

            for edge in outgoing {
                // Apply label filter if specified
                if let Some(ref filter) = label_filter {
                    if !filter.contains(edge.label()) {
                        continue;
                    }
                }

                let target = edge.target();

                // Add traversal result
                results.push(TraversalResult {
                    depth: depth + 1,
                    source: current_node,
                    target,
                    label: edge.label().to_string(),
                    edge_id: edge.id(),
                });

                // Check memory bound
                if results.len() >= config.max_visited {
                    return Ok(results);
                }

                // Queue unvisited nodes
                if !visited.contains(&target) {
                    visited.insert(target);
                    queue.push_back((target, depth + 1));
                }
            }
        }

        Ok(results)
    }

    /// Removes an edge by ID.
    #[pyo3(signature = (edge_id))]
    fn remove_edge(&self, edge_id: u64) -> PyResult<()> {
        let mut store = self
            .inner
            .write()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
        store.remove_edge(edge_id);
        Ok(())
    }

    /// Returns the number of edges in the store.
    fn edge_count(&self) -> PyResult<usize> {
        let store = self
            .inner
            .read()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
        Ok(store.edge_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_config_defaults() {
        let config = StreamingConfig::new(3, 10000, None);
        assert_eq!(config.max_depth, 3);
        assert_eq!(config.max_visited, 10000);
        assert!(config.relationship_types.is_none());
    }
}
