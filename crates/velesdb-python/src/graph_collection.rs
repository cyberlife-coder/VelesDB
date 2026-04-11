//! Persistent `GraphCollection` bindings for VelesDB Python.
//!
//! Wraps `velesdb_core::GraphCollection` (disk-backed, persistent graph)
//! as a `PyGraphCollection` pyclass.  Follows the same patterns as
//! `crate::collection::Collection` (error handling, dict conversion).

use pyo3::prelude::*;
use std::collections::HashMap;

use crate::collection_helpers::{core_err, point_to_dict, search_result_to_dict};
use crate::graph::{dict_to_edge, edge_to_dict, traversal_to_dict};
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
#[pyclass(name = "GraphSchema", frozen)]
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
    pub(crate) inner: GraphCollection,
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
    fn add_edge(&self, py: Python<'_>, edge: HashMap<String, PyObject>) -> PyResult<()> {
        let graph_edge = dict_to_edge(py, &edge)?;
        py.allow_threads(|| self.inner.add_edge(graph_edge).map_err(core_err))
    }

    /// Add multiple edges in batch (much faster than calling add_edge in a loop).
    ///
    /// Defers CSR snapshot rebuild until after all edges are inserted,
    /// eliminating per-edge rebuild overhead.
    ///
    /// Args:
    ///     edges: List of edge dicts (same format as add_edge)
    ///
    /// Returns:
    ///     Number of edges successfully added
    ///
    /// Example:
    ///     >>> graph.add_edges_batch([
    ///     ...     {"id": 1, "source": 10, "target": 20, "label": "KNOWS"},
    ///     ...     {"id": 2, "source": 20, "target": 30, "label": "FOLLOWS"},
    ///     ... ])
    #[pyo3(signature = (edges))]
    fn add_edges_batch(
        &self,
        py: Python<'_>,
        edges: Vec<HashMap<String, PyObject>>,
    ) -> PyResult<usize> {
        let graph_edges: Vec<velesdb_core::collection::graph::GraphEdge> = edges
            .iter()
            .map(|e| dict_to_edge(py, e))
            .collect::<PyResult<Vec<_>>>()?;
        py.allow_threads(|| Ok(self.inner.add_edges_batch(graph_edges)))
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
    fn get_edges(&self, py: Python<'_>, label: Option<String>) -> PyResult<Vec<PyObject>> {
        let edges = py.allow_threads(|| self.inner.get_edges(label.as_deref()));
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
    }

    /// Get outgoing edges from a node.
    ///
    /// Args:
    ///     node_id: The source node ID
    ///
    /// Returns:
    ///     List of edge dicts
    #[pyo3(signature = (node_id))]
    fn get_outgoing(&self, py: Python<'_>, node_id: u64) -> PyResult<Vec<PyObject>> {
        let edges = py.allow_threads(|| self.inner.get_outgoing(node_id));
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
    }

    /// Get incoming edges to a node.
    ///
    /// Args:
    ///     node_id: The target node ID
    ///
    /// Returns:
    ///     List of edge dicts
    #[pyo3(signature = (node_id))]
    fn get_incoming(&self, py: Python<'_>, node_id: u64) -> PyResult<Vec<PyObject>> {
        let edges = py.allow_threads(|| self.inner.get_incoming(node_id));
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
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

    /// Upsert the payload (properties) for a node.
    ///
    /// Replaces any pre-existing payload on the given node, following
    /// the standard VelesDB upsert semantics. Renamed from
    /// `store_node_payload` in v1.13 to match the Rust core API and
    /// the rest of the Python surface (which uses `upsert` everywhere).
    ///
    /// Args:
    ///     node_id: The node ID
    ///     payload: Dict of properties to store
    ///
    /// Example:
    ///     >>> graph.upsert_node_payload(10, {"name": "Alice", "age": 30})
    #[pyo3(signature = (node_id, payload))]
    fn upsert_node_payload(&self, py: Python<'_>, node_id: u64, payload: PyObject) -> PyResult<()> {
        let value = python_to_json(py, &payload)?;
        py.allow_threads(|| {
            self.inner
                .upsert_node_payload(node_id, &value)
                .map_err(core_err)
        })
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
        let value = py.allow_threads(|| self.inner.get_node_payload(node_id).map_err(core_err))?;
        Ok(value.map(|v| json_to_python(py, &v)))
    }

    /// Get all node IDs that have a stored payload.
    ///
    /// Returns:
    ///     List of node IDs
    fn all_node_ids(&self, py: Python<'_>) -> Vec<u64> {
        py.allow_threads(|| self.inner.all_node_ids())
    }

    /// Perform BFS traversal from a source node.
    ///
    /// Args:
    ///     source_id: Starting node ID
    ///     max_depth: Maximum traversal depth (default: 3)
    ///     limit: Maximum results to return (default: 100)
    ///     rel_types: Optional list of relationship types to follow.
    ///         Alias: ``relationship_types`` (same effect, either name works).
    ///
    /// Returns:
    ///     List of traversal result dicts with keys: target_id, path, depth
    ///
    /// Example:
    ///     >>> results = graph.traverse_bfs(source_id=1, max_depth=3)
    ///     >>> results = graph.traverse_bfs(1, rel_types=["KNOWS"])
    ///     >>> results = graph.traverse_bfs(1, relationship_types=["KNOWS"])  # alias
    #[pyo3(signature = (source_id, max_depth=None, limit=None, rel_types=None, relationship_types=None))]
    fn traverse_bfs(
        &self,
        py: Python<'_>,
        source_id: u64,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
        relationship_types: Option<Vec<String>>,
    ) -> PyResult<Vec<PyObject>> {
        let effective_rel_types = rel_types.or(relationship_types);
        let config = build_traversal_config(max_depth, limit, effective_rel_types);
        let results = py.allow_threads(|| self.inner.traverse_bfs(source_id, &config));
        Ok(results.iter().map(|r| traversal_to_dict(py, r)).collect())
    }

    /// Perform DFS traversal from a source node.
    ///
    /// Args:
    ///     source_id: Starting node ID
    ///     max_depth: Maximum traversal depth (default: 3)
    ///     limit: Maximum results to return (default: 100)
    ///     rel_types: Optional list of relationship types to follow.
    ///         Alias: ``relationship_types`` (same effect, either name works).
    ///
    /// Returns:
    ///     List of traversal result dicts with keys: target_id, path, depth
    ///
    /// Example:
    ///     >>> results = graph.traverse_dfs(source_id=1, max_depth=3)
    ///     >>> results = graph.traverse_dfs(1, rel_types=["KNOWS"])
    ///     >>> results = graph.traverse_dfs(1, relationship_types=["KNOWS"])  # alias
    #[pyo3(signature = (source_id, max_depth=None, limit=None, rel_types=None, relationship_types=None))]
    fn traverse_dfs(
        &self,
        py: Python<'_>,
        source_id: u64,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
        relationship_types: Option<Vec<String>>,
    ) -> PyResult<Vec<PyObject>> {
        let effective_rel_types = rel_types.or(relationship_types);
        let config = build_traversal_config(max_depth, limit, effective_rel_types);
        let results = py.allow_threads(|| self.inner.traverse_dfs(source_id, &config));
        Ok(results.iter().map(|r| traversal_to_dict(py, r)).collect())
    }

    /// Perform multi-source BFS traversal with deduplication.
    ///
    /// Starts BFS from multiple source nodes simultaneously and deduplicates
    /// results by path signature.
    ///
    /// Args:
    ///     source_ids: List of starting node IDs
    ///     max_depth: Maximum traversal depth (default: 3)
    ///     limit: Maximum results to return (default: 100)
    ///     rel_types: Optional list of relationship types to follow.
    ///         Alias: ``relationship_types`` (same effect, either name works).
    ///
    /// Returns:
    ///     List of traversal result dicts with keys: target_id, path, depth
    ///
    /// Example:
    ///     >>> results = graph.traverse_bfs_parallel([1, 5, 10], max_depth=3)
    #[pyo3(signature = (source_ids, max_depth=None, limit=None, rel_types=None, relationship_types=None))]
    fn traverse_bfs_parallel(
        &self,
        py: Python<'_>,
        source_ids: Vec<u64>,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
        relationship_types: Option<Vec<String>>,
    ) -> PyResult<Vec<PyObject>> {
        let effective_rel_types = rel_types.or(relationship_types);
        let config = build_traversal_config(max_depth, limit, effective_rel_types);
        let results = py.allow_threads(|| self.inner.traverse_bfs_parallel(&source_ids, &config));
        Ok(results.iter().map(|r| traversal_to_dict(py, r)).collect())
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
    ) -> PyResult<Vec<PyObject>> {
        let vec = extract_vector(py, &query)?;
        let top_k = k.unwrap_or(10);

        let results = py.allow_threads(|| {
            self.inner
                .search_by_embedding(&vec, top_k)
                .map_err(core_err)
        })?;

        Ok(results
            .iter()
            .map(|r| search_result_to_dict(py, r))
            .collect())
    }

    /// Flush all graph state to disk.
    ///
    /// Ensures edges, payloads, and indexes are persisted.
    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        py.allow_threads(|| self.inner.flush().map_err(core_err))
    }

    /// Returns the total number of edges in the graph.
    ///
    /// Returns:
    ///     int: Edge count
    fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }

    /// Full durability flush including WAL serialization.
    ///
    /// Use on graceful shutdown to avoid a full WAL replay on next startup.
    /// For routine persistence, use ``flush()`` instead.
    fn flush_full(&self, py: Python<'_>) -> PyResult<()> {
        py.allow_threads(|| self.inner.flush_full().map_err(core_err))
    }

    /// Returns the number of points (nodes with payload) in the graph.
    ///
    /// Returns:
    ///     int: Point count
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Returns the number of points (nodes with payload) in the graph.
    ///
    /// Returns:
    ///     int: Point count
    fn count(&self) -> usize {
        self.inner.len()
    }

    /// Check if the graph collection has no stored points.
    ///
    /// Returns:
    ///     bool: True if empty
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get points by their IDs.
    ///
    /// Args:
    ///     ids: List of point IDs to retrieve
    ///
    /// Returns:
    ///     List of point dicts (or None for missing IDs)
    #[pyo3(signature = (ids))]
    fn get(&self, py: Python<'_>, ids: Vec<u64>) -> PyResult<Vec<Option<PyObject>>> {
        let points = py.allow_threads(|| self.inner.get(&ids));
        let py_points = points
            .into_iter()
            .map(|opt_point| opt_point.map(|p| point_to_dict(py, &p)))
            .collect();
        Ok(py_points)
    }

    /// Delete points by their IDs.
    ///
    /// Args:
    ///     ids: List of point IDs to delete
    #[pyo3(signature = (ids))]
    fn delete(&self, py: Python<'_>, ids: Vec<u64>) -> PyResult<()> {
        py.allow_threads(|| self.inner.delete(&ids).map_err(core_err))
    }

    /// Remove a specific edge by its ID.
    ///
    /// Args:
    ///     edge_id: The edge ID to remove
    ///
    /// Returns:
    ///     bool: True if the edge existed and was removed
    #[pyo3(signature = (edge_id))]
    fn remove_edge(&self, py: Python<'_>, edge_id: u64) -> bool {
        py.allow_threads(|| self.inner.remove_edge(edge_id))
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
// VelesQL query methods (parity with Collection) — moved to graph_collection_query.rs
// ---------------------------------------------------------------------------

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
