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
use std::collections::HashMap;
use std::sync::Arc;

use crate::graph::{dict_to_edge, edge_to_dict};
use velesdb_core::collection::graph::EdgeStore;
// FLAG-1 FIX: Use core's BfsIterator instead of re-implementing BFS
use velesdb_core::collection::graph::{bfs_stream, StreamingConfig as CoreStreamingConfig};

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
    ///     List of TraversalResult objects representing edge traversals.
    ///
    /// Note:
    ///     - The start node itself is NOT included in results (it has no incoming edge).
    ///     - Results represent edges traversed, not nodes visited.
    ///     - Results are bounded by config.max_visited to prevent memory exhaustion.
    ///     - To get the start node, query it separately before traversal.
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

        // FLAG-1 FIX: Use core's BfsIterator instead of re-implementing BFS
        // Convert Python config to core config
        let rel_types: Vec<String> = config.relationship_types.unwrap_or_default();

        #[allow(clippy::cast_possible_truncation)]
        let core_config = CoreStreamingConfig {
            max_depth: config.max_depth as u32,
            max_visited_size: config.max_visited,
            rel_types,
            limit: Some(config.max_visited),
        };

        // Use core's bfs_stream iterator
        let iterator = bfs_stream(&store, start_node, core_config);

        // Collect results, converting from core TraversalResult to Python TraversalResult
        // FLAG-2 FIX: Handle empty path correctly instead of using unwrap_or(0)
        // which could collide with a real edge_id=0
        let results: Vec<TraversalResult> = iterator
            .take(config.max_visited)
            .filter_map(|r| {
                // Get edge info from the path - skip results with empty path
                let edge_id = r.path.last().copied()?;
                let edge = store.get_edge(edge_id);
                let (source, label) = edge
                    .map(|e| (e.source(), e.label().to_string()))
                    .unwrap_or((start_node, String::new()));

                Some(TraversalResult {
                    depth: r.depth as usize,
                    source,
                    target: r.target_id,
                    label,
                    edge_id,
                })
            })
            .collect();

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

    /// Checks if an edge exists.
    ///
    /// Args:
    ///     edge_id: The edge ID to check
    ///
    /// Returns:
    ///     True if the edge exists, False otherwise.
    #[pyo3(signature = (edge_id))]
    fn has_edge(&self, edge_id: u64) -> PyResult<bool> {
        let store = self
            .inner
            .read()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
        Ok(store.get_edge(edge_id).is_some())
    }

    /// Gets the out-degree (number of outgoing edges) of a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///
    /// Returns:
    ///     Number of outgoing edges from this node.
    #[pyo3(signature = (node_id))]
    fn out_degree(&self, node_id: u64) -> PyResult<usize> {
        let store = self
            .inner
            .read()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
        Ok(store.get_outgoing(node_id).len())
    }

    /// Gets the in-degree (number of incoming edges) of a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///
    /// Returns:
    ///     Number of incoming edges to this node.
    #[pyo3(signature = (node_id))]
    fn in_degree(&self, node_id: u64) -> PyResult<usize> {
        let store = self
            .inner
            .read()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;
        Ok(store.get_incoming(node_id).len())
    }

    /// Performs DFS traversal from a source node.
    ///
    /// Args:
    ///     source_id: Starting node ID
    ///     config: StreamingConfig with max_depth, max_visited, relationship_types
    ///
    /// Returns:
    ///     List of TraversalResult objects for each edge visited.
    ///
    /// Example:
    ///     >>> results = store.traverse_dfs(100, StreamingConfig(max_depth=3))
    ///     >>> for r in results:
    ///     ...     print(f"Depth {r.depth}: {r.source} -> {r.target}")
    #[pyo3(signature = (source_id, config))]
    fn traverse_dfs(
        &self,
        source_id: u64,
        config: &StreamingConfig,
    ) -> PyResult<Vec<TraversalResult>> {
        use std::collections::HashSet;

        let store = self
            .inner
            .read()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {e}")))?;

        let mut results = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut stack: Vec<(u64, usize)> = vec![(source_id, 0)];

        while let Some((node_id, depth)) = stack.pop() {
            if visited.len() >= config.max_visited {
                break;
            }

            if visited.contains(&node_id) {
                continue;
            }
            visited.insert(node_id);

            if depth < config.max_depth {
                let edges = store.get_outgoing(node_id);
                let filtered: Vec<_> = edges
                    .into_iter()
                    .filter(|e| {
                        if let Some(ref types) = config.relationship_types {
                            types.contains(&e.label().to_string())
                        } else {
                            true
                        }
                    })
                    .filter(|e| !visited.contains(&e.target()))
                    .collect();

                for edge in filtered.into_iter().rev() {
                    results.push(TraversalResult {
                        depth: depth + 1,
                        source: edge.source(),
                        target: edge.target(),
                        label: edge.label().to_string(),
                        edge_id: edge.id(),
                    });
                    stack.push((edge.target(), depth + 1));
                }
            }
        }

        Ok(results)
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
