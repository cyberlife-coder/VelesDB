//! `GraphCollection`: knowledge graph with optional node embeddings.
//!
//! # Design
//!
//! `GraphCollection` is a pure newtype over `Collection` (C-02).
//! All graph state (edge store, property/range indexes, node payloads, optional
//! HNSW for node embeddings) lives inside the single `inner: Collection`.
//! The graph schema and embedding dimension are persisted in `config.json`.
//! There are no separate engine fields — no dual-storage risk.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::collection::graph::{GraphEdge, GraphSchema, TraversalConfig, TraversalResult};
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::point::SearchResult;

/// A graph collection storing typed relationships between nodes.
///
/// Node embeddings are optional: if `dimension` is `None`, no vector index is created.
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::{GraphCollection, GraphSchema, GraphEdge, DistanceMetric};
///
/// let coll = GraphCollection::create(
///     "./data/kg".into(),
///     "knowledge",
///     None,                    // no embeddings
///     DistanceMetric::Cosine,  // unused when no embeddings
///     GraphSchema::schemaless(),
/// )?;
///
/// let edge = GraphEdge::new(1, 100, 200, "KNOWS")?;
/// coll.add_edge(edge)?;
/// # Ok::<(), velesdb_core::Error>(())
/// ```
#[derive(Clone)]
pub struct GraphCollection {
    /// Single source of truth — all graph state lives here (C-02 pure newtype).
    pub(crate) inner: Collection,
}

impl GraphCollection {
    // -------------------------------------------------------------------------
    // Lifecycle
    // -------------------------------------------------------------------------

    /// Creates a new `GraphCollection`.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create(
        path: PathBuf,
        name: &str,
        dimension: Option<usize>,
        metric: DistanceMetric,
        schema: GraphSchema,
    ) -> Result<Self> {
        Ok(Self {
            inner: Collection::create_graph_collection(path, name, schema, dimension, metric)?,
        })
    }

    /// Opens an existing `GraphCollection` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if config or storage cannot be opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        Ok(Self {
            inner: Collection::open(path)?,
        })
    }

    /// Flushes all state to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush(&self) -> Result<()> {
        self.inner.flush()
    }

    // -------------------------------------------------------------------------
    // Metadata
    // -------------------------------------------------------------------------

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.inner.config().name
    }

    /// Returns the graph schema stored in config.
    ///
    /// Returns `GraphSchema::schemaless()` for collections that have no schema set.
    #[must_use]
    pub fn schema(&self) -> GraphSchema {
        self.inner
            .graph_schema()
            .unwrap_or_else(GraphSchema::schemaless)
    }

    /// Returns `true` if this collection stores node embeddings.
    #[must_use]
    pub fn has_embeddings(&self) -> bool {
        self.inner.has_embeddings()
    }

    // -------------------------------------------------------------------------
    // Graph operations — delegate to Collection graph API
    // -------------------------------------------------------------------------

    /// Adds an edge between two nodes.
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    pub fn add_edge(&self, edge: GraphEdge) -> Result<()> {
        self.inner.add_edge(edge)
    }

    /// Returns edges, optionally filtered by label.
    #[must_use]
    pub fn get_edges(&self, label: Option<&str>) -> Vec<GraphEdge> {
        match label {
            Some(lbl) => self.inner.get_edges_by_label(lbl),
            None => self.inner.get_all_edges(),
        }
    }

    /// Returns all outgoing edges from a node.
    #[must_use]
    pub fn get_outgoing(&self, node_id: u64) -> Vec<GraphEdge> {
        self.inner.get_outgoing_edges(node_id)
    }

    /// Returns all incoming edges to a node.
    #[must_use]
    pub fn get_incoming(&self, node_id: u64) -> Vec<GraphEdge> {
        self.inner.get_incoming_edges(node_id)
    }

    /// Returns `(in_degree, out_degree)` for a node.
    #[must_use]
    pub fn node_degree(&self, node_id: u64) -> (usize, usize) {
        self.inner.get_node_degree(node_id)
    }

    /// Returns the IDs of all nodes that have a stored payload.
    ///
    /// Nodes that appear only as edge endpoints without a stored payload
    /// are not included. Use [`GraphCollection::get_edges`] to discover
    /// all referenced node IDs.
    #[must_use]
    pub fn all_node_ids(&self) -> Vec<u64> {
        self.inner.all_ids()
    }

    /// Performs BFS traversal from a source node.
    #[must_use]
    pub fn traverse_bfs(&self, source_id: u64, config: &TraversalConfig) -> Vec<TraversalResult> {
        self.inner.traverse_bfs_config(source_id, config)
    }

    /// Performs DFS traversal from a source node.
    #[must_use]
    pub fn traverse_dfs(&self, source_id: u64, config: &TraversalConfig) -> Vec<TraversalResult> {
        self.inner.traverse_dfs_config(source_id, config)
    }

    // -------------------------------------------------------------------------
    // Payload / node properties
    // -------------------------------------------------------------------------

    /// Stores node payload (properties).
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn store_node_payload(&self, node_id: u64, payload: &serde_json::Value) -> Result<()> {
        self.inner.store_node_payload(node_id, payload)
    }

    /// Retrieves node payload.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval fails.
    pub fn get_node_payload(&self, node_id: u64) -> Result<Option<serde_json::Value>> {
        self.inner.get_node_payload(node_id)
    }

    // -------------------------------------------------------------------------
    // Optional embedding search
    // -------------------------------------------------------------------------

    /// Searches for similar nodes by embedding (only available if `has_embeddings()`).
    ///
    /// # Errors
    ///
    /// Returns `Error::VectorNotAllowed` if this collection has no embeddings,
    /// or `Error::DimensionMismatch` if the query dimension is wrong.
    pub fn search_by_embedding(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        self.inner.search_by_embedding(query, k)
    }

    // -------------------------------------------------------------------------
    // VelesQL
    // -------------------------------------------------------------------------

    /// Executes a `VelesQL` query.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is invalid or execution fails.
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.execute_query(query, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::graph::GraphSchema;
    use crate::distance::DistanceMetric;
    use tempfile::tempdir;

    #[test]
    fn test_all_node_ids_returns_ids_with_payload() {
        let dir = tempdir().unwrap();
        let col = GraphCollection::create(
            dir.path().to_path_buf(),
            "kg",
            None,
            DistanceMetric::Cosine,
            GraphSchema::schemaless(),
        )
        .unwrap();

        // Store payloads on two nodes
        col.store_node_payload(10, &serde_json::json!({"name": "Alice"}))
            .unwrap();
        col.store_node_payload(20, &serde_json::json!({"name": "Bob"}))
            .unwrap();

        let ids = col.all_node_ids();
        assert!(ids.contains(&10), "node 10 should be present");
        assert!(ids.contains(&20), "node 20 should be present");
        assert_eq!(ids.len(), 2);
    }
}
