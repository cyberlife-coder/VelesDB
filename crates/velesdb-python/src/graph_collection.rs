//! Persistent `GraphCollection` bindings for VelesDB Python.
//!
//! Wraps `velesdb_core::GraphCollection` (disk-backed, persistent graph)
//! as a `PyGraphCollection` pyclass.  Follows the same patterns as
//! `crate::collection::Collection` (error handling, dict conversion).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::collection::{deny_if_scoped, retain_by_filters};
use crate::collection_helpers::{core_err, point_to_dict, search_result_to_dict};
use crate::graph::{dict_to_edge, edge_to_dict, traversal_to_dict};
use crate::utils::{extract_vector, json_to_python, python_to_json};
use velesdb_core::collection::graph::TraversalConfig;
use velesdb_core::{Database as CoreDatabase, GraphCollection, GraphSchema, QueryOperationKind};

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
#[pyclass(name = "GraphSchema", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyGraphSchema {
    pub(crate) inner: GraphSchema,
}

impl PyGraphSchema {
    /// Wraps a core `GraphSchema`.
    pub(crate) fn from_inner(inner: GraphSchema) -> Self {
        Self { inner }
    }

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
    /// Shared handle to the owning database. Like `Collection`, `inner` is a
    /// detached leaf with no observer reference, so `search_by_embedding` routes
    /// its vector read through this handle's control-plane gate rather than
    /// hitting the leaf directly.
    pub(crate) db: Arc<CoreDatabase>,
    pub(crate) name: String,
}

impl PyGraphCollection {
    /// Creates a new `PyGraphCollection` wrapper.
    pub fn new(inner: GraphCollection, db: Arc<CoreDatabase>, name: String) -> Self {
        Self { inner, db, name }
    }

    /// Routes a read through the database's read gate, as `GraphTraversal` —
    /// the operation every edge/traversal read on a graph collection is.
    /// A scope filter fails closed (`deny_if_scoped`): no graph read has a
    /// metadata-filtered leaf to narrow, mirroring `MobileGraphStore`.
    fn authorize(&self, context: &str) -> PyResult<()> {
        let scope = self
            .db
            .authorize_read(&self.name, QueryOperationKind::GraphTraversal, None, None)
            .map_err(core_err)?;
        deny_if_scoped(scope, context)
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
    /// The schema is collection state, so the read gate decides it like the
    /// gated metadata getters on ``Collection`` (#2589).
    ///
    /// Returns:
    ///     GraphSchema: The schema configuration
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (fail closed).
    #[getter]
    fn schema(&self) -> PyResult<PyGraphSchema> {
        self.authorize("schema")?;
        Ok(PyGraphSchema::from_inner(self.inner.schema()))
    }

    /// Whether this collection has node embeddings enabled.
    ///
    /// Embeddings availability is collection state, so the read gate decides
    /// it (#2589).
    ///
    /// Returns:
    ///     bool: True if vector search is available
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (fail closed).
    #[getter]
    fn has_embeddings(&self) -> PyResult<bool> {
        self.authorize("has_embeddings")?;
        Ok(self.inner.has_embeddings())
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
    fn add_edge(&self, py: Python<'_>, edge: HashMap<String, Py<PyAny>>) -> PyResult<()> {
        let graph_edge = dict_to_edge(py, &edge)?;
        py.detach(|| self.inner.add_edge(graph_edge).map_err(core_err))
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
        edges: Vec<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<usize> {
        let graph_edges: Vec<velesdb_core::collection::graph::GraphEdge> = edges
            .iter()
            .map(|e| dict_to_edge(py, e))
            .collect::<PyResult<Vec<_>>>()?;
        py.detach(|| self.inner.add_edges_batch(graph_edges).map_err(core_err))
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
    fn get_edges(&self, py: Python<'_>, label: Option<String>) -> PyResult<Vec<Py<PyAny>>> {
        self.authorize("get_edges")?;
        let edges = py.detach(|| self.inner.get_edges(label.as_deref()));
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
    fn get_outgoing(&self, py: Python<'_>, node_id: u64) -> PyResult<Vec<Py<PyAny>>> {
        self.authorize("get_outgoing")?;
        let edges = py.detach(|| self.inner.get_outgoing(node_id));
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
    }

    /// Returns `true` if an edge with the given ID exists.
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (fail closed).
    #[pyo3(signature = (edge_id))]
    fn has_edge(&self, edge_id: u64) -> PyResult<bool> {
        self.authorize("has_edge")?;
        Ok(self.inner.has_edge(edge_id))
    }

    /// Alias for `get_outgoing`.
    ///
    /// Preserves compatibility with examples that used the explicit
    /// `*_edges` suffix.
    #[pyo3(signature = (node_id))]
    fn get_outgoing_edges(&self, py: Python<'_>, node_id: u64) -> PyResult<Vec<Py<PyAny>>> {
        self.get_outgoing(py, node_id)
    }

    /// Get incoming edges to a node.
    ///
    /// Args:
    ///     node_id: The target node ID
    ///
    /// Returns:
    ///     List of edge dicts
    #[pyo3(signature = (node_id))]
    fn get_incoming(&self, py: Python<'_>, node_id: u64) -> PyResult<Vec<Py<PyAny>>> {
        self.authorize("get_incoming")?;
        let edges = py.detach(|| self.inner.get_incoming(node_id));
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
    }

    /// Alias for `get_incoming`.
    ///
    /// Preserves compatibility with examples that used the explicit
    /// `*_edges` suffix.
    #[pyo3(signature = (node_id))]
    fn get_incoming_edges(&self, py: Python<'_>, node_id: u64) -> PyResult<Vec<Py<PyAny>>> {
        self.get_incoming(py, node_id)
    }

    /// Get a single edge by its ID.
    ///
    /// Args:
    ///     edge_id: The edge ID to look up
    ///
    /// Returns:
    ///     Edge dict (id, source, target, label, properties), or None
    #[pyo3(signature = (edge_id))]
    fn get_edge(&self, py: Python<'_>, edge_id: u64) -> PyResult<Option<Py<PyAny>>> {
        self.authorize("get_edge")?;
        let edge = py.detach(|| self.inner.get_edge(edge_id));
        Ok(edge.map(|e| edge_to_dict(py, &e)))
    }

    /// Bounded outgoing-edge read: at most ``cap`` edges plus the true total
    /// out-degree, in O(cap) even for super-nodes (#1820).
    ///
    /// Returns:
    ///     Dict with keys ``edges`` (list of edge dicts) and ``total`` (int)
    #[pyo3(signature = (node_id, cap))]
    fn get_outgoing_bounded(
        &self,
        py: Python<'_>,
        node_id: u64,
        cap: usize,
    ) -> PyResult<Py<PyAny>> {
        self.authorize("get_outgoing_bounded")?;
        let (edges, total) = py.detach(|| self.inner.get_outgoing_bounded(node_id, cap));
        bounded_edges_to_dict(py, edges, total)
    }

    /// Bounded incoming-edge read: at most ``cap`` edges plus the true total
    /// in-degree, in O(cap) even for super-nodes (#1820).
    ///
    /// Returns:
    ///     Dict with keys ``edges`` (list of edge dicts) and ``total`` (int)
    #[pyo3(signature = (node_id, cap))]
    fn get_incoming_bounded(
        &self,
        py: Python<'_>,
        node_id: u64,
        cap: usize,
    ) -> PyResult<Py<PyAny>> {
        self.authorize("get_incoming_bounded")?;
        let (edges, total) = py.detach(|| self.inner.get_incoming_bounded(node_id, cap));
        bounded_edges_to_dict(py, edges, total)
    }

    /// Remove every edge, then every node (the same rule VelesQL `TRUNCATE`
    /// follows). Not atomic: removals that succeeded before a failure stay
    /// applied, and the error reports the counts.
    ///
    /// Returns:
    ///     Tuple of (edges_removed, nodes_removed)
    fn truncate(&self, py: Python<'_>) -> PyResult<(usize, usize)> {
        py.detach(|| self.inner.truncate().map_err(core_err))
    }

    /// Remove an edge by ID, distinguishing "absent" from "WAL refused".
    ///
    /// Unlike :py:meth:`remove_edge` (which returns False on both), this
    /// raises `VelesDBError` (VELES-006) when the removal was attempted and
    /// the write-ahead log refused it, leaving the edge in place.
    ///
    /// Returns:
    ///     bool: True if the edge existed and was removed, False if absent
    #[pyo3(signature = (edge_id))]
    fn try_remove_edge(&self, py: Python<'_>, edge_id: u64) -> PyResult<bool> {
        py.detach(|| self.inner.try_remove_edge(edge_id).map_err(core_err))
    }

    /// Get the in-degree and out-degree of a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///
    /// Returns:
    ///     Tuple of (in_degree, out_degree)
    #[pyo3(signature = (node_id))]
    fn node_degree(&self, node_id: u64) -> PyResult<(usize, usize)> {
        self.authorize("node_degree")?;
        Ok(self.inner.node_degree(node_id))
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
    fn upsert_node_payload(
        &self,
        py: Python<'_>,
        node_id: u64,
        payload: Py<PyAny>,
    ) -> PyResult<()> {
        let value = python_to_json(py, &payload)?;
        py.detach(|| {
            self.inner
                .upsert_node_payload(node_id, &value)
                .map_err(core_err)
        })?;
        Ok(())
    }

    /// Batch node-payload upsert: one WAL barrier, and the whole batch is
    /// validated before anything is written, so a rejected batch commits
    /// nothing (#2153).
    ///
    /// Args:
    ///     entries: List of dicts with keys ``id`` (int) and ``payload`` (dict)
    ///
    /// Example:
    ///     >>> graph.upsert_node_payloads([
    ///     ...     {"id": 10, "payload": {"name": "Alice"}},
    ///     ...     {"id": 20, "payload": {"name": "Bob"}},
    ///     ... ])
    #[pyo3(signature = (entries))]
    fn upsert_node_payloads(
        &self,
        py: Python<'_>,
        entries: Vec<HashMap<String, Py<PyAny>>>,
    ) -> PyResult<()> {
        let mut parsed: Vec<(u64, serde_json::Value)> = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let id = crate::collection_helpers::extract_point_id(py, entry, index)?;
            let payload = crate::collection_helpers::parse_payload(py, entry.get("payload"))?
                .ok_or_else(|| {
                    PyValueError::new_err(format!(
                        "entry at index {index} must have a 'payload' field"
                    ))
                })?;
            parsed.push((id, payload));
        }
        py.detach(|| {
            let refs: Vec<(u64, &serde_json::Value)> =
                parsed.iter().map(|(id, value)| (*id, value)).collect();
            self.inner.upsert_node_payloads(&refs).map_err(core_err)
        })?;
        Ok(())
    }

    /// Scroll graph nodes in ascending-ID batches (stateless one-shot, same
    /// shape as :py:meth:`Collection.scroll_batch`).
    ///
    /// Args:
    ///     cursor: Resume after this node ID (None to start from the beginning)
    ///     batch_size: Maximum nodes per batch (default: 100)
    ///     filter: Optional metadata filter dict (same shape as search filters)
    ///
    /// Returns:
    ///     Tuple ``(points, next_cursor)``: the batch as a list of point dicts
    ///     and the cursor for the next call (the last point's ID; ``None`` once
    ///     the batch is empty)
    #[pyo3(signature = (cursor=None, batch_size=100, filter=None))]
    fn scroll_batch(
        &self,
        py: Python<'_>,
        cursor: Option<u64>,
        batch_size: usize,
        filter: Option<Py<PyAny>>,
    ) -> PyResult<(Vec<Py<PyAny>>, Option<u64>)> {
        self.authorize("scroll_batch")?;
        let filter_obj = crate::collection_helpers::parse_optional_filter(py, filter)?;
        let batch = py.detach(|| {
            self.inner
                .scroll_batch(cursor, batch_size, filter_obj.as_ref())
                .map_err(core_err)
        })?;
        let points: Vec<Py<PyAny>> = batch.points.iter().map(|p| point_to_dict(py, p)).collect();
        Ok((points, batch.next_cursor))
    }

    /// Operational graph metrics (edge/traversal counters).
    ///
    /// Returns:
    ///     Dict with keys: edges_total, edge_inserts_total, edge_deletes_total,
    ///     traversals_total, traversal_nodes_visited
    fn metrics(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.authorize("metrics")?;
        let (edges_total, inserts, deletes, traversals, visited) = py.detach(|| {
            let m = self.inner.metrics();
            (
                m.edges_total(),
                m.edge_inserts_total(),
                m.edge_deletes_total(),
                m.traversals_total(),
                m.traversal_nodes_visited(),
            )
        });
        let out = PyDict::new(py);
        let _ = out.set_item("edges_total", edges_total);
        let _ = out.set_item("edge_inserts_total", inserts);
        let _ = out.set_item("edge_deletes_total", deletes);
        let _ = out.set_item("traversals_total", traversals);
        let _ = out.set_item("traversal_nodes_visited", visited);
        Ok(out.into_any().unbind())
    }

    /// Upsert a node payload, optionally with an embedding vector.
    ///
    /// Args:
    ///     node_id: The node ID
    ///     payload: Dict of properties to store
    ///     vector: Optional embedding vector for searchable graph nodes
    #[pyo3(signature = (node_id, payload, vector=None))]
    fn upsert_node(
        &self,
        py: Python<'_>,
        node_id: u64,
        payload: Py<PyAny>,
        vector: Option<Py<PyAny>>,
    ) -> PyResult<()> {
        let value = python_to_json(py, &payload)?;
        let vector = match vector {
            Some(vector) => Some(extract_vector(py, &vector)?),
            None => None,
        };
        py.detach(|| {
            self.inner
                .upsert_node(node_id, &value, vector)
                .map_err(core_err)
        })?;
        Ok(())
    }

    /// Retrieve payload (properties) for a node.
    ///
    /// Args:
    ///     node_id: The node ID
    ///
    /// Returns:
    ///     Dict of properties, or None if no payload is stored
    #[pyo3(signature = (node_id))]
    fn get_node_payload(&self, py: Python<'_>, node_id: u64) -> PyResult<Option<Py<PyAny>>> {
        self.authorize("get_node_payload")?;
        let value = py.detach(|| self.inner.get_node_payload(node_id).map_err(core_err))?;
        Ok(value.map(|v| json_to_python(py, &v)))
    }

    /// Get all node IDs that have a stored payload.
    ///
    /// Returns:
    ///     List of node IDs
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (ids carry no
    ///         payload to filter — fail closed).
    fn all_node_ids(&self, py: Python<'_>) -> PyResult<Vec<u64>> {
        self.authorize("all_node_ids")?;
        Ok(py.detach(|| self.inner.all_node_ids()))
    }

    /// Perform BFS traversal from a source node.
    ///
    /// Args:
    ///     source_id: Starting node ID
    ///     max_depth: Maximum traversal depth (default: 3)
    ///     limit: Maximum results to return (default: 100)
    ///     rel_types: Optional list of relationship types to follow.
    ///         Alias: ``relationship_types`` (same effect, either name works).
    ///     min_depth: Minimum hops (inclusive) to include (default: 1)
    ///     timeout_ms: Wall-clock budget in milliseconds; when it expires the
    ///         partial result accumulated so far is returned (default: none)
    ///
    /// Returns:
    ///     List of traversal result dicts with keys: target_id, path, depth
    ///
    /// Example:
    ///     >>> results = graph.traverse_bfs(source_id=1, max_depth=3)
    ///     >>> results = graph.traverse_bfs(1, rel_types=["KNOWS"])
    ///     >>> results = graph.traverse_bfs(1, relationship_types=["KNOWS"])  # alias
    #[allow(clippy::too_many_arguments)] // traversal knobs thread through one config
    #[pyo3(signature = (source_id, max_depth=None, limit=None, rel_types=None, relationship_types=None, min_depth=None, timeout_ms=None))]
    fn traverse_bfs(
        &self,
        py: Python<'_>,
        source_id: u64,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
        relationship_types: Option<Vec<String>>,
        min_depth: Option<u32>,
        timeout_ms: Option<u64>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        self.authorize("traverse_bfs")?;
        let effective_rel_types = rel_types.or(relationship_types);
        let config =
            build_traversal_config(min_depth, max_depth, limit, effective_rel_types, timeout_ms);
        let results = py.detach(|| self.inner.traverse_bfs(source_id, &config));
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
    #[allow(clippy::too_many_arguments)] // traversal knobs thread through one config
    #[pyo3(signature = (source_id, max_depth=None, limit=None, rel_types=None, relationship_types=None, min_depth=None, timeout_ms=None))]
    fn traverse_dfs(
        &self,
        py: Python<'_>,
        source_id: u64,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
        relationship_types: Option<Vec<String>>,
        min_depth: Option<u32>,
        timeout_ms: Option<u64>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        self.authorize("traverse_dfs")?;
        let effective_rel_types = rel_types.or(relationship_types);
        let config =
            build_traversal_config(min_depth, max_depth, limit, effective_rel_types, timeout_ms);
        let results = py.detach(|| self.inner.traverse_dfs(source_id, &config));
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
    #[allow(clippy::too_many_arguments)] // traversal knobs thread through one config
    #[pyo3(signature = (source_ids, max_depth=None, limit=None, rel_types=None, relationship_types=None, min_depth=None, timeout_ms=None))]
    fn traverse_bfs_parallel(
        &self,
        py: Python<'_>,
        source_ids: Vec<u64>,
        max_depth: Option<u32>,
        limit: Option<usize>,
        rel_types: Option<Vec<String>>,
        relationship_types: Option<Vec<String>>,
        min_depth: Option<u32>,
        timeout_ms: Option<u64>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        self.authorize("traverse_bfs_parallel")?;
        let effective_rel_types = rel_types.or(relationship_types);
        let config =
            build_traversal_config(min_depth, max_depth, limit, effective_rel_types, timeout_ms);
        let results = py.detach(|| self.inner.traverse_bfs_parallel(&source_ids, &config));
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
    ///     PermissionError: If the read gate denies the read.
    ///
    /// The `principal` argument is caller-supplied and only meaningful when a
    /// trusted embedder forwards a verified identity (local-SDK trust boundary):
    /// the gate cannot itself authenticate the caller.
    #[pyo3(signature = (query, k=None, *, principal=None, tenant=None))]
    fn search_by_embedding(
        &self,
        py: Python<'_>,
        query: Py<PyAny>,
        k: Option<usize>,
        principal: Option<String>,
        tenant: Option<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let vec = extract_vector(py, &query)?;
        let top_k = k.unwrap_or(10);

        // Route the vector read through the control-plane gate. The graph leaf
        // takes no metadata filter, so authorize first (Deny ⇒ Err, fail closed)
        // then apply any observer scope filter as a post-filter (narrow-only),
        // mirroring the sparse / parallel-batch paths on `Collection`.
        let results = py.detach(|| {
            let scope = self
                .db
                .authorize_read(
                    &self.name,
                    QueryOperationKind::VectorSearch,
                    principal.as_deref(),
                    tenant.as_deref(),
                )
                .map_err(core_err)?;
            let mut results = self
                .inner
                .search_by_embedding(&vec, top_k)
                .map_err(core_err)?;
            if let Some(scope_filter) = scope {
                retain_by_filters(&mut results, None, Some(&scope_filter));
            }
            Ok::<_, PyErr>(results)
        })?;

        Ok(results
            .iter()
            .map(|r| search_result_to_dict(py, r, false))
            .collect())
    }

    /// Flush all graph state to disk.
    ///
    /// Ensures edges, payloads, and indexes are persisted.
    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.flush().map_err(core_err))
    }

    /// Returns the total number of edges in the graph.
    ///
    /// Returns:
    ///     int: Edge count
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (fail closed).
    fn edge_count(&self) -> PyResult<usize> {
        self.authorize("edge_count")?;
        Ok(self.inner.edge_count())
    }

    /// Full durability flush including WAL serialization.
    ///
    /// Use on graceful shutdown to avoid a full WAL replay on next startup.
    /// For routine persistence, use ``flush()`` instead.
    fn flush_full(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.flush_full().map_err(core_err))
    }

    /// Returns the number of points (nodes with payload) in the graph.
    ///
    /// Returns:
    ///     int: Point count
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (fail closed).
    fn __len__(&self) -> PyResult<usize> {
        self.authorize("__len__")?;
        Ok(self.inner.len())
    }

    /// Returns the number of points (nodes with payload) in the graph,
    /// through the same read gate as ``__len__``.
    ///
    /// Returns:
    ///     int: Point count
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (fail closed).
    fn count(&self) -> PyResult<usize> {
        self.authorize("count")?;
        Ok(self.inner.len())
    }

    /// Check if the graph collection has no stored points.
    ///
    /// Returns:
    ///     bool: True if empty
    ///
    /// Raises:
    ///     PermissionError: If the read gate denies the read.
    ///     PermissionError: If the read gate returns a scope (fail closed).
    fn is_empty(&self) -> PyResult<bool> {
        self.authorize("is_empty")?;
        Ok(self.inner.is_empty())
    }

    /// Get points by their IDs.
    ///
    /// Args:
    ///     ids: List of point IDs to retrieve
    ///
    /// Returns:
    ///     List of point dicts (or None for missing IDs)
    #[pyo3(signature = (ids))]
    fn get(&self, py: Python<'_>, ids: Vec<u64>) -> PyResult<Vec<Option<Py<PyAny>>>> {
        self.authorize("get")?;
        let points = py.detach(|| self.inner.get(&ids));
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
        py.detach(|| self.inner.delete(&ids).map_err(core_err))
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
        py.detach(|| self.inner.remove_edge(edge_id))
    }

    fn __repr__(&self) -> PyResult<String> {
        // The repr leaks `has_embeddings`, a gated read (#2589).
        self.authorize("__repr__")?;
        Ok(format!(
            "GraphCollection(name='{}', has_embeddings={})",
            self.name,
            self.inner.has_embeddings(),
        ))
    }

    /// Membership test: ``node_id in graph``.
    ///
    /// Args:
    ///     node_id: The node ID to look up
    ///
    /// Returns:
    ///     bool: True if a node with that ID has a stored payload
    ///
    /// Note: signature must omit ``py: Python<'_>`` so PyO3 installs this
    /// as the ``sq_contains`` slot. Uses ``get_node_payload`` because
    /// graph nodes are payload-addressed (not vector-addressed like
    /// :py:class:`Collection`).
    fn __contains__(&self, node_id: u64) -> PyResult<bool> {
        // Membership reveals node existence, so it is a gated read (same
        // rationale as Collection.__contains__).
        self.authorize("__contains__")?;
        Ok(matches!(self.inner.get_node_payload(node_id), Ok(Some(_))))
    }

    /// Graceful shutdown: full durability flush including WAL serialization.
    ///
    /// Idempotent — safe to call multiple times. Equivalent to
    /// :py:meth:`flush_full` but named so graph collections can be used
    /// as a context manager.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.flush_full().map_err(core_err))
    }

    /// Context manager entry — returns ``self``.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context manager exit — calls :py:meth:`close` and re-raises any
    /// exception raised inside the ``with`` block.
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: Option<Py<PyAny>>,
        _exc_value: Option<Py<PyAny>>,
        _traceback: Option<Py<PyAny>>,
    ) -> PyResult<bool> {
        self.close(py)?;
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// VelesQL query methods (parity with Collection) — moved to graph_collection_query.rs
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `TraversalConfig` from optional Python parameters.
///
/// `timeout_ms` becomes a wall-clock deadline (`Instant::now() + timeout`);
/// when it expires mid-traversal, core returns the partial result accumulated
/// so far — the same convention as the `limit` guard.
fn build_traversal_config(
    min_depth: Option<u32>,
    max_depth: Option<u32>,
    limit: Option<usize>,
    rel_types: Option<Vec<String>>,
    timeout_ms: Option<u64>,
) -> TraversalConfig {
    TraversalConfig {
        min_depth: min_depth.unwrap_or(1),
        max_depth: max_depth.unwrap_or(3),
        limit: limit.unwrap_or(100),
        rel_types: rel_types.unwrap_or_default(),
        deadline: timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms)),
    }
}

/// Render a bounded edge read as ``{"edges": [...], "total": n}``.
fn bounded_edges_to_dict(
    py: Python<'_>,
    edges: Vec<velesdb_core::collection::graph::GraphEdge>,
    total: usize,
) -> PyResult<Py<PyAny>> {
    let out = PyDict::new(py);
    let list = PyList::empty(py);
    for edge in &edges {
        let _ = list.append(edge_to_dict(py, edge));
    }
    let _ = out.set_item("edges", list);
    let _ = out.set_item("total", total);
    Ok(out.into_any().unbind())
}

#[cfg(test)]
#[path = "graph_collection_tests.rs"]
mod tests;
