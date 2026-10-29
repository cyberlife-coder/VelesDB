//! Graph bindings for VelesDB Mobile (UniFFI).
//!
//! [`MobileGraphStore`] is a handle on a core [`GraphCollection`]: nodes are
//! that collection's node payloads, edges live in its edge store, and every
//! traversal is core's own. The binding implements no graph storage and no
//! traversal; it maps the UniFFI records to and from core's types
//! (`graph_payload.rs`) and routes each read through the owning database's
//! read gate.

use std::sync::Arc;

use velesdb_core::{
    Database as CoreDatabase, GraphCollection, QueryOperationKind, TraversalConfig,
};

use crate::collection::deny_if_scoped;
use crate::graph_payload::{edge_to_core, node_from_payload, node_has_label, node_payload};
use crate::VelesError;

/// A graph node for knowledge graph construction.
#[derive(Debug, Clone, uniffi::Record, serde::Serialize, serde::Deserialize)]
pub struct MobileGraphNode {
    /// Unique identifier.
    pub id: u64,
    /// Node type/label.
    pub label: String,
    /// JSON properties as string.
    pub properties_json: Option<String>,
    /// Optional vector embedding.
    pub vector: Option<Vec<f32>>,
}

/// A graph edge representing a relationship.
#[derive(Debug, Clone, uniffi::Record, serde::Serialize, serde::Deserialize)]
pub struct MobileGraphEdge {
    /// Unique identifier.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Relationship type.
    pub label: String,
    /// JSON properties as string.
    pub properties_json: Option<String>,
}

/// Traversal result from BFS/DFS.
///
/// FFI projection of [`velesdb_core::TraversalResult`]. The mobile field
/// `node_id` corresponds to core's `target_id` (the node reached); `path`
/// and `depth` mirror core's fields one-for-one. See the `From` impls below
/// for the canonical mapping — they make any future core field drift a
/// compile error rather than silent divergence.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraversalResult {
    /// Target node ID reached (core: `target_id`).
    pub node_id: u64,
    /// Edge IDs along the path from the source to this node (core: `path`).
    pub path: Vec<u64>,
    /// Depth from source (number of hops).
    pub depth: u32,
}

/// Serializes a core property map to the mobile `properties_json` shape.
///
/// Returns `None` for an empty map (no properties) and a JSON object string
/// otherwise. A serialization failure also yields `None` so the projection is
/// total (FFI conversions cannot return a `Result`).
fn properties_to_json(
    properties: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    if properties.is_empty() {
        return None;
    }
    serde_json::to_string(properties).ok()
}

impl From<velesdb_core::GraphNode> for MobileGraphNode {
    fn from(node: velesdb_core::GraphNode) -> Self {
        Self {
            id: node.id(),
            label: node.label().to_string(),
            properties_json: properties_to_json(node.properties()),
            vector: node.vector().cloned(),
        }
    }
}

impl From<velesdb_core::GraphEdge> for MobileGraphEdge {
    fn from(edge: velesdb_core::GraphEdge) -> Self {
        Self {
            id: edge.id(),
            source: edge.source(),
            target: edge.target(),
            label: edge.label().to_string(),
            properties_json: properties_to_json(edge.properties()),
        }
    }
}

impl From<velesdb_core::TraversalResult> for TraversalResult {
    fn from(result: velesdb_core::TraversalResult) -> Self {
        Self {
            node_id: result.target_id,
            path: result.path,
            depth: result.depth,
        }
    }
}

/// Maps a batch of core edges to their UniFFI records.
fn to_mobile_edges(edges: Vec<velesdb_core::GraphEdge>) -> Vec<MobileGraphEdge> {
    edges.into_iter().map(MobileGraphEdge::from).collect()
}

/// Maps a batch of core traversal results to their UniFFI records.
fn to_mobile_results(results: Vec<velesdb_core::TraversalResult>) -> Vec<TraversalResult> {
    results.into_iter().map(TraversalResult::from).collect()
}

/// Saturates a core count to the `u32` the FFI surface exposes.
fn saturating_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The knowledge graph of one graph collection.
///
/// Obtained from [`VelesDatabase::get_graph_store`](crate::VelesDatabase::get_graph_store)
/// after [`create_graph_collection`](crate::VelesDatabase::create_graph_collection).
/// Storage is core's: writes are durable as core makes them (edge WAL, node
/// payload log), so the graph survives an app restart with no explicit save,
/// and a node written here is the node VelesQL `MATCH` sees through
/// [`execute_query`](crate::VelesDatabase::execute_query). BFS, DFS and
/// multi-source BFS are core's [`GraphCollection`] traversals.
///
/// Every read consults the database's read gate as a `GraphTraversal` and
/// fails closed on an observer scope filter, which no graph read can apply.
#[derive(uniffi::Object)]
pub struct MobileGraphStore {
    inner: GraphCollection,
    /// Shared handle to the owning database, carrying the read gate.
    db: Arc<CoreDatabase>,
    /// Collection name, used to address the gate.
    name: String,
}

impl MobileGraphStore {
    /// Wraps a core graph collection owned by `db` under `name`.
    pub(crate) fn new(inner: GraphCollection, db: Arc<CoreDatabase>, name: String) -> Self {
        Self { inner, db, name }
    }

    /// Routes a read through the database's read gate. A scope filter fails
    /// closed: no graph read has a metadata-filtered leaf to narrow.
    fn authorize(&self, context: &str) -> Result<(), VelesError> {
        let scope =
            self.db
                .authorize_read(&self.name, QueryOperationKind::GraphTraversal, None, None)?;
        deny_if_scoped(scope, context)
    }

    /// Core traversal settings for one mobile `(max_depth, limit)` pair. The
    /// source itself (depth 0) is never emitted, as in core's default.
    fn traversal_config(max_depth: u32, limit: u32) -> TraversalConfig {
        TraversalConfig::with_range(1, max_depth)
            .with_limit(usize::try_from(limit).unwrap_or(usize::MAX))
    }

    /// Reads node `id`.
    fn read_node(&self, id: u64) -> Result<Option<MobileGraphNode>, VelesError> {
        Ok(self
            .inner
            .get_node_payload(id)?
            .map(|payload| self.assemble_node(id, payload)))
    }

    /// Builds node `id` from its payload, with its embedding when the
    /// collection stores them.
    fn assemble_node(&self, id: u64, payload: serde_json::Value) -> MobileGraphNode {
        let vector = if self.inner.has_embeddings() {
            self.inner
                .get(&[id])
                .into_iter()
                .flatten()
                .next()
                .map(|point| point.vector)
        } else {
            None
        };
        node_from_payload(id, payload, vector)
    }
}

#[uniffi::export]
impl MobileGraphStore {
    /// Adds a node, or replaces the node with the same ID.
    ///
    /// The label is stored as core's node label, so VelesQL `MATCH (n:Label)`
    /// finds the node. `properties_json` must be a JSON object and must not
    /// carry the reserved `_labels` key. A `vector` requires a collection
    /// created with embeddings, of the matching dimension.
    pub fn add_node(&self, node: MobileGraphNode) -> Result<(), VelesError> {
        let payload = node_payload(&node)?;
        self.inner.upsert_node(node.id, &payload, node.vector)?;
        Ok(())
    }

    /// Adds an edge.
    ///
    /// Fails when the edge ID is taken, when the label is empty, or when the
    /// source or target node has not been added (core's referential check).
    pub fn add_edge(&self, edge: MobileGraphEdge) -> Result<(), VelesError> {
        self.inner.add_edge(edge_to_core(&edge)?)?;
        Ok(())
    }

    /// Imports a JSON snapshot written by the removed `MobileGraphStore.save`.
    ///
    /// The migration path from the in-memory store of earlier releases: load
    /// the file once into this store, after which core persists it. Nodes
    /// are upserted; an edge whose ID this store already holds is refused
    /// rather than overwritten, as is an edge whose endpoint node is in
    /// neither the snapshot nor this store (core requires both endpoints;
    /// the old store did not), so add such nodes first, then import. These
    /// refusals happen before the first write.
    pub fn import_snapshot(&self, path: String) -> Result<(), VelesError> {
        crate::graph_payload::import_snapshot(&self.inner, &path)
    }

    /// Flushes the graph's pending writes to disk.
    pub fn flush(&self) -> Result<(), VelesError> {
        self.inner.flush()?;
        Ok(())
    }

    /// Gets a node by ID.
    pub fn get_node(&self, id: u64) -> Result<Option<MobileGraphNode>, VelesError> {
        self.authorize("get_node")?;
        self.read_node(id)
    }

    /// Gets an edge by ID.
    pub fn get_edge(&self, id: u64) -> Result<Option<MobileGraphEdge>, VelesError> {
        self.authorize("get_edge")?;
        Ok(self.inner.get_edge(id).map(MobileGraphEdge::from))
    }

    /// Returns the number of nodes.
    pub fn node_count(&self) -> Result<u64, VelesError> {
        self.authorize("node_count")?;
        Ok(self.inner.all_node_ids().len() as u64)
    }

    /// Returns the number of edges.
    pub fn edge_count(&self) -> Result<u64, VelesError> {
        self.authorize("edge_count")?;
        Ok(self.inner.edge_count() as u64)
    }

    /// Gets outgoing edges from a node.
    pub fn get_outgoing(&self, node_id: u64) -> Result<Vec<MobileGraphEdge>, VelesError> {
        self.authorize("get_outgoing")?;
        Ok(to_mobile_edges(self.inner.get_outgoing(node_id)))
    }

    /// Gets incoming edges to a node.
    pub fn get_incoming(&self, node_id: u64) -> Result<Vec<MobileGraphEdge>, VelesError> {
        self.authorize("get_incoming")?;
        Ok(to_mobile_edges(self.inner.get_incoming(node_id)))
    }

    /// Gets outgoing edges filtered by label.
    pub fn get_outgoing_by_label(
        &self,
        node_id: u64,
        label: String,
    ) -> Result<Vec<MobileGraphEdge>, VelesError> {
        let mut edges = self.get_outgoing(node_id)?;
        edges.retain(|e| e.label == label);
        Ok(edges)
    }

    /// Gets neighbors reachable from a node (1-hop).
    pub fn get_neighbors(&self, node_id: u64) -> Result<Vec<u64>, VelesError> {
        Ok(self
            .get_outgoing(node_id)?
            .into_iter()
            .map(|e| e.target)
            .collect())
    }

    /// Performs core's BFS from a source node.
    ///
    /// # Arguments
    ///
    /// * `source_id` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `limit` - Maximum number of results
    pub fn bfs_traverse(
        &self,
        source_id: u64,
        max_depth: u32,
        limit: u32,
    ) -> Result<Vec<TraversalResult>, VelesError> {
        self.authorize("bfs_traverse")?;
        let config = Self::traversal_config(max_depth, limit);
        Ok(to_mobile_results(
            self.inner.traverse_bfs(source_id, &config),
        ))
    }

    /// Performs core's multi-source BFS.
    ///
    /// Each source is traversed independently; results are deduplicated by
    /// path, so a node reached from two sources is reported once per source.
    ///
    /// # Arguments
    ///
    /// * `source_ids` - Starting node IDs
    /// * `max_depth` - Maximum traversal depth
    /// * `limit` - Maximum number of results
    pub fn bfs_traverse_parallel(
        &self,
        source_ids: Vec<u64>,
        max_depth: u32,
        limit: u32,
    ) -> Result<Vec<TraversalResult>, VelesError> {
        self.authorize("bfs_traverse_parallel")?;
        let config = Self::traversal_config(max_depth, limit);
        Ok(to_mobile_results(
            self.inner.traverse_bfs_parallel(&source_ids, &config),
        ))
    }

    /// Performs core's DFS from a source node.
    ///
    /// # Arguments
    ///
    /// * `source_id` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `limit` - Maximum number of results
    pub fn dfs_traverse(
        &self,
        source_id: u64,
        max_depth: u32,
        limit: u32,
    ) -> Result<Vec<TraversalResult>, VelesError> {
        self.authorize("dfs_traverse")?;
        let config = Self::traversal_config(max_depth, limit);
        Ok(to_mobile_results(
            self.inner.traverse_dfs(source_id, &config),
        ))
    }

    /// Removes a node and every edge connected to it.
    pub fn remove_node(&self, node_id: u64) -> Result<(), VelesError> {
        self.inner.delete(&[node_id])?;
        Ok(())
    }

    /// Removes an edge by ID, returning whether there was one to remove.
    ///
    /// Removing an absent edge is a no-op that returns `false`. A removal the
    /// edge write-ahead log refuses leaves the edge in place and throws core's
    /// storage error (`VELES-006`), as does an index found out of sync with
    /// the edge's source shard.
    pub fn remove_edge(&self, edge_id: u64) -> Result<bool, VelesError> {
        Ok(self.inner.try_remove_edge(edge_id)?)
    }

    /// Removes every node and edge.
    ///
    /// The edges go first and the nodes only once none is left, the order
    /// core's `TRUNCATE` follows: deleting the nodes of an edge whose removal
    /// failed would leave that edge dangling. Every edge is attempted, and the
    /// first failure is the one reported.
    pub fn clear(&self) -> Result<(), VelesError> {
        let mut first_failure = None;
        for edge in self.inner.get_edges(None) {
            if let Err(err) = self.inner.try_remove_edge(edge.id()) {
                first_failure.get_or_insert(err);
            }
        }
        if let Some(err) = first_failure {
            return Err(err.into());
        }
        self.inner.delete(&self.inner.all_node_ids())?;
        Ok(())
    }

    /// Checks if a node exists.
    pub fn has_node(&self, id: u64) -> Result<bool, VelesError> {
        self.authorize("has_node")?;
        Ok(self.inner.get_node_payload(id)?.is_some())
    }

    /// Checks if an edge exists.
    pub fn has_edge(&self, id: u64) -> Result<bool, VelesError> {
        self.authorize("has_edge")?;
        Ok(self.inner.has_edge(id))
    }

    /// Gets the out-degree (number of outgoing edges) of a node.
    pub fn out_degree(&self, node_id: u64) -> Result<u32, VelesError> {
        self.authorize("out_degree")?;
        Ok(saturating_u32(self.inner.node_degree(node_id).1))
    }

    /// Gets the in-degree (number of incoming edges) of a node.
    pub fn in_degree(&self, node_id: u64) -> Result<u32, VelesError> {
        self.authorize("in_degree")?;
        Ok(saturating_u32(self.inner.node_degree(node_id).0))
    }

    /// Gets all nodes carrying `label`.
    pub fn get_nodes_by_label(&self, label: String) -> Result<Vec<MobileGraphNode>, VelesError> {
        self.authorize("get_nodes_by_label")?;
        let mut nodes = Vec::new();
        for id in self.inner.all_node_ids() {
            let Some(payload) = self.inner.get_node_payload(id)? else {
                continue;
            };
            if node_has_label(&payload, &label) {
                nodes.push(self.assemble_node(id, payload));
            }
        }
        Ok(nodes)
    }

    /// Gets all edges with a specific label.
    pub fn get_edges_by_label(&self, label: String) -> Result<Vec<MobileGraphEdge>, VelesError> {
        self.authorize("get_edges_by_label")?;
        Ok(to_mobile_edges(self.inner.get_edges(Some(&label))))
    }
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
