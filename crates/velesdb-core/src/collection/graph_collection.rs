//! `GraphCollection`: knowledge graph with optional node embeddings.
//!
//! # Design
//!
//! `GraphCollection` is a pure newtype over `Collection` (C-02).
//! All graph state (edge store, property/range indexes, node payloads, optional
//! HNSW for node embeddings) lives inside the single `inner: Collection`.
//! The graph schema and embedding dimension are persisted in `config.json`.
//! There are no separate engine fields — no dual-storage risk.

use std::path::PathBuf;

use crate::collection::graph::{GraphEdge, GraphSchema, TraversalConfig, TraversalResult};
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::point::{Point, SearchResult};

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

    /// Consumes `self` and returns a [`VectorCollection`](super::VectorCollection)
    /// **structural view** over this graph collection's shared `inner` store.
    ///
    /// This is a purely structural re-wrap of the identical `inner: Collection`
    /// backing store into the `VectorCollection` newtype — an ordinary value
    /// move, **not** a `transmute` and not memory-unsafe. It does **not** assert
    /// that the collection is vector-kind: invoking vector-specific methods on
    /// the result returns empty or misleading state.
    ///
    /// It exists solely for the **Python binding**, whose single user-facing
    /// `Collection` type is backed by a `VectorCollection`: the binding holds a
    /// graph collection behind that type while gating vector-only operations on
    /// the real kind it tracks separately (the Python
    /// `Collection::ensure_vector` guard). The Mobile and Tauri bindings do
    /// *not* use this view — they go through the variant-checked
    /// [`AnyCollection::into_vector`](super::AnyCollection::into_vector) and
    /// reject non-vector collections. Callers that need the graph surface must
    /// use the graph API, not this view.
    ///
    /// [`MetadataCollection::into_vector_view`](super::MetadataCollection::into_vector_view)
    /// is the exact mirror for metadata collections.
    #[must_use]
    pub fn into_vector_view(self) -> super::VectorCollection {
        super::VectorCollection { inner: self.inner }
    }

    /// Flushes all state to disk.
    ///
    /// Issue #423: This fast-path flush skips `vectors.idx` serialization.
    /// The WAL provides crash recovery for the vector index.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush(&self) -> Result<()> {
        self.inner.flush()
    }

    /// Full durability flush including `vectors.idx` serialization.
    ///
    /// Issue #423: Use on graceful shutdown to avoid a full WAL replay
    /// on the next startup.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush_full(&self) -> Result<()> {
        self.inner.flush_full()
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
    /// - Returns `Error::EdgeExists` if an edge with the same ID already exists.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{GraphCollection, GraphSchema, GraphEdge, DistanceMetric};
    /// # let coll = GraphCollection::create("./data/kg".into(), "kg", None, DistanceMetric::Cosine, GraphSchema::schemaless())?;
    /// let edge = GraphEdge::new(1, 100, 200, "KNOWS")?;
    /// coll.add_edge(edge)?;
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn add_edge(&self, edge: GraphEdge) -> Result<()> {
        self.inner.add_edge(edge)
    }

    /// Adds multiple edges in batch (much faster than calling add_edge in a loop).
    ///
    /// Acquires locks once for the entire batch and rebuilds the CSR snapshot
    /// once at the end. Duplicate edge IDs are silently skipped.
    ///
    /// # Returns
    ///
    /// Number of edges successfully added.
    ///
    /// # Errors
    ///
    /// Returns an error if WAL durability logging fails for graph
    /// collections (fail-closed: the in-memory store is not mutated).
    pub fn add_edges_batch(&self, edges: Vec<GraphEdge>) -> Result<usize> {
        self.inner.add_edges_batch(edges)
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

    /// Returns the total number of edges in the graph without materializing them.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.inner.edge_count()
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

    /// Returns the next batch of points for scroll iteration.
    ///
    /// Delegates to the inner collection's `scroll_batch` (parallel
    /// implementation to [`VectorCollection::scroll_batch`](crate::VectorCollection::scroll_batch)).
    ///
    /// # Errors
    ///
    /// Returns an error if `batch_size` is 0.
    pub fn scroll_batch(
        &self,
        cursor: Option<u64>,
        batch_size: usize,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<crate::collection::ScrollBatch> {
        self.inner.scroll_batch(cursor, batch_size, filter)
    }

    /// Returns the number of nodes (points) stored in this collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the collection contains no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Retrieves nodes by IDs, returning `None` for missing entries.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        self.inner.get(ids)
    }

    /// Deletes nodes by IDs.
    ///
    /// Missing IDs are silently ignored.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        self.inner.delete(ids)
    }

    /// Removes an edge from the graph by ID.
    ///
    /// Returns `true` if the edge existed and was removed, `false` otherwise.
    #[must_use]
    pub fn remove_edge(&self, edge_id: u64) -> bool {
        self.inner.remove_edge(edge_id)
    }

    /// Returns `true` if an edge with `edge_id` exists in the graph.
    #[must_use]
    pub fn has_edge(&self, edge_id: u64) -> bool {
        self.inner.edge_exists(edge_id)
    }

    /// Performs BFS traversal from a source node.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{GraphCollection, GraphSchema, GraphEdge, DistanceMetric};
    /// # use velesdb_core::collection::graph::TraversalConfig;
    /// # let coll = GraphCollection::create("./data/kg".into(), "kg", None, DistanceMetric::Cosine, GraphSchema::schemaless())?;
    /// let config = TraversalConfig { max_depth: 3, ..TraversalConfig::default() };
    /// let results = coll.traverse_bfs(100, &config);
    /// for r in &results {
    ///     println!("node={} depth={}", r.target_id, r.depth);
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    #[must_use]
    pub fn traverse_bfs(&self, source_id: u64, config: &TraversalConfig) -> Vec<TraversalResult> {
        self.inner.traverse_bfs_config(source_id, config)
    }

    /// Performs DFS traversal from a source node.
    #[must_use]
    pub fn traverse_dfs(&self, source_id: u64, config: &TraversalConfig) -> Vec<TraversalResult> {
        self.inner.traverse_dfs_config(source_id, config)
    }

    /// Performs parallel BFS traversal from multiple start nodes.
    ///
    /// When `start_nodes` exceeds the parallel threshold (100 nodes), rayon
    /// distributes independent per-start-node BFS traversals across CPU cores.
    /// Results are deduplicated by path signature and truncated to `config.limit`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{GraphCollection, GraphSchema, DistanceMetric};
    /// # use velesdb_core::collection::graph::TraversalConfig;
    /// # let coll = GraphCollection::create("./data/kg".into(), "kg", None, DistanceMetric::Cosine, GraphSchema::schemaless())?;
    /// let config = TraversalConfig { max_depth: 3, ..TraversalConfig::default() };
    /// let results = coll.traverse_bfs_parallel(&[100, 200, 300], &config);
    /// for r in &results {
    ///     println!("node={} depth={}", r.target_id, r.depth);
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    #[must_use]
    pub fn traverse_bfs_parallel(
        &self,
        start_nodes: &[u64],
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        self.inner.traverse_bfs_parallel(start_nodes, config)
    }

    // -------------------------------------------------------------------------
    // Payload / node properties
    // -------------------------------------------------------------------------

    /// Inserts or updates node payload (properties).
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn upsert_node_payload(&self, node_id: u64, payload: &serde_json::Value) -> Result<()> {
        self.inner.store_node_payload(node_id, payload)
    }

    /// Inserts or updates a node payload, optionally with an embedding vector.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails, the vector dimension is invalid, or
    /// an embedding is supplied for a graph collection without embeddings.
    pub fn upsert_node(
        &self,
        node_id: u64,
        payload: &serde_json::Value,
        vector: Option<Vec<f32>>,
    ) -> Result<()> {
        match vector {
            Some(vector) => self
                .inner
                .upsert([Point::new(node_id, vector, Some(payload.clone()))]),
            None => self.upsert_node_payload(node_id, payload),
        }
    }

    /// Inserts or updates node payload (properties).
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    #[deprecated(since = "1.6.0", note = "Use upsert_node_payload() instead")]
    pub fn store_node_payload(&self, node_id: u64, payload: &serde_json::Value) -> Result<()> {
        self.upsert_node_payload(node_id, payload)
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

    /// Alias for [`search_by_embedding`](Self::search_by_embedding).
    ///
    /// Provided for API parity with [`crate::VectorCollection::search`].
    ///
    /// # Errors
    ///
    /// Returns `Error::VectorNotAllowed` if this collection has no embeddings,
    /// or `Error::DimensionMismatch` if the query dimension is wrong.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        self.search_by_embedding(query, k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::graph::GraphSchema;
    use crate::distance::DistanceMetric;
    use std::collections::HashMap;
    use tempfile::{tempdir, TempDir};

    /// Creates a schemaless cosine `GraphCollection` in a fresh temp dir.
    ///
    /// Returns the `TempDir` guard alongside the collection so the backing
    /// directory outlives the test. `dimension` controls embedding support
    /// (`None` for payload/edge-only collections, `Some(n)` for searchable ones).
    fn make_test_collection(dimension: Option<usize>) -> (TempDir, GraphCollection) {
        let dir = tempdir().unwrap();
        let col = GraphCollection::create(
            dir.path().to_path_buf(),
            "kg",
            dimension,
            DistanceMetric::Cosine,
            GraphSchema::schemaless(),
        )
        .unwrap();
        (dir, col)
    }

    #[test]
    fn test_all_node_ids_returns_ids_with_payload() {
        let (_dir, col) = make_test_collection(None);

        // Store payloads on two nodes
        col.upsert_node_payload(10, &serde_json::json!({"name": "Alice"}))
            .unwrap();
        col.upsert_node_payload(20, &serde_json::json!({"name": "Bob"}))
            .unwrap();

        let ids = col.all_node_ids();
        assert!(ids.contains(&10), "node 10 should be present");
        assert!(ids.contains(&20), "node 20 should be present");
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn test_upsert_node_with_embedding_is_searchable() {
        let (_dir, col) = make_test_collection(Some(4));

        col.upsert_node(
            10,
            &serde_json::json!({"name": "Alice"}),
            Some(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .unwrap();

        assert_eq!(
            col.get_node_payload(10).unwrap(),
            Some(serde_json::json!({"name": "Alice"}))
        );
        let results = col.search_by_embedding(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results[0].point.id, 10);
    }

    #[test]
    fn test_edge_count_returns_correct_count() {
        let (_dir, col) = make_test_collection(None);

        assert_eq!(col.edge_count(), 0);
        for id in [10, 20, 30] {
            col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
        }

        let edge1 = crate::collection::graph::GraphEdge::new(1, 10, 20, "knows").unwrap();
        col.add_edge(edge1).unwrap();
        assert_eq!(col.edge_count(), 1);

        let edge2 = crate::collection::graph::GraphEdge::new(2, 20, 30, "likes").unwrap();
        col.add_edge(edge2).unwrap();
        assert_eq!(col.edge_count(), 2);
    }

    #[test]
    fn test_traverse_bfs_parallel_through_graph_collection() {
        let (_dir, col) = make_test_collection(None);

        // Build chain: 1->2->3
        for id in [1, 2, 3] {
            col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
        }
        col.add_edge(GraphEdge::new(1, 1, 2, "NEXT").unwrap())
            .unwrap();
        col.add_edge(GraphEdge::new(2, 2, 3, "NEXT").unwrap())
            .unwrap();

        let config = TraversalConfig {
            max_depth: 3,
            min_depth: 1,
            ..TraversalConfig::default()
        };
        let results = col.traverse_bfs_parallel(&[1], &config);
        let target_ids: std::collections::HashSet<u64> =
            results.iter().map(|r| r.target_id).collect();
        assert!(target_ids.contains(&2), "should reach node 2");
        assert!(target_ids.contains(&3), "should reach node 3");
    }

    #[test]
    fn test_execute_match_finds_edges() {
        let (_dir, col) = make_test_collection(None);

        // Store node payloads with labels
        col.upsert_node_payload(
            10,
            &serde_json::json!({"_labels": ["Person"], "name": "Alice"}),
        )
        .unwrap();
        col.upsert_node_payload(
            20,
            &serde_json::json!({"_labels": ["Person"], "name": "Bob"}),
        )
        .unwrap();

        // Add edge: Alice -> Bob
        let edge = crate::collection::graph::GraphEdge::new(1, 10, 20, "KNOWS").unwrap();
        col.add_edge(edge).unwrap();

        // MATCH query through the GraphCollection delegate
        let match_clause = crate::velesql::MatchClause {
            patterns: vec![crate::velesql::GraphPattern {
                name: None,
                nodes: vec![
                    crate::velesql::NodePattern::new().with_alias("a"),
                    crate::velesql::NodePattern::new().with_alias("b"),
                ],
                relationships: vec![crate::velesql::RelationshipPattern::new(
                    crate::velesql::Direction::Outgoing,
                )],
            }],
            where_clause: None,
            return_clause: crate::velesql::ReturnClause {
                items: vec![],
                order_by: None,
                limit: Some(10),
            },
        };

        let params = HashMap::new();
        let results = col.execute_match(&match_clause, &params).unwrap();
        assert!(
            !results.is_empty(),
            "execute_match should find the KNOWS edge"
        );
        assert_eq!(results[0].node_id, 20, "target should be Bob (id=20)");
    }

    #[test]
    fn test_has_edge_and_remove_edge() {
        let (_dir, col) = make_test_collection(None);
        assert!(!col.has_edge(7), "unknown edge id is absent");
        for id in [10, 20] {
            col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
        }

        col.add_edge(GraphEdge::new(7, 10, 20, "KNOWS").unwrap())
            .unwrap();
        assert!(col.has_edge(7), "edge present after add");

        assert!(col.remove_edge(7), "removing an existing edge returns true");
        assert!(!col.has_edge(7), "edge gone after remove");
        assert!(!col.remove_edge(7), "removing a missing edge returns false");
    }

    #[test]
    fn test_upsert_node_without_vector_stores_payload_only() {
        // No embeddings: the `None`-vector branch delegates to upsert_node_payload.
        let (_dir, col) = make_test_collection(None);
        col.upsert_node(42, &serde_json::json!({"name": "Carol"}), None)
            .unwrap();
        assert_eq!(
            col.get_node_payload(42).unwrap(),
            Some(serde_json::json!({"name": "Carol"}))
        );
        assert!(col.all_node_ids().contains(&42));
        assert!(!col.has_embeddings(), "no embeddings without a dimension");
    }

    #[test]
    fn test_get_edges_filtered_by_label() {
        let (_dir, col) = make_test_collection(None);
        for id in [10, 20, 30, 40] {
            col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
        }
        col.add_edge(GraphEdge::new(1, 10, 20, "KNOWS").unwrap())
            .unwrap();
        col.add_edge(GraphEdge::new(2, 20, 30, "LIKES").unwrap())
            .unwrap();
        col.add_edge(GraphEdge::new(3, 30, 40, "KNOWS").unwrap())
            .unwrap();

        let knows = col.get_edges(Some("KNOWS"));
        assert_eq!(knows.len(), 2, "two KNOWS edges");
        assert!(knows.iter().all(|e| e.label() == "KNOWS"));

        let all = col.get_edges(None);
        assert_eq!(all.len(), 3, "three edges total");
    }

    #[test]
    fn test_node_degree_and_directional_edges() {
        let (_dir, col) = make_test_collection(None);
        for id in [10, 20, 30, 40] {
            col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
        }
        col.add_edge(GraphEdge::new(1, 10, 20, "NEXT").unwrap())
            .unwrap();
        col.add_edge(GraphEdge::new(2, 30, 20, "NEXT").unwrap())
            .unwrap();
        col.add_edge(GraphEdge::new(3, 20, 40, "NEXT").unwrap())
            .unwrap();

        // Node 20: 2 incoming (from 10, 30), 1 outgoing (to 40).
        assert_eq!(col.node_degree(20), (2, 1));
        assert_eq!(col.get_incoming(20).len(), 2);
        let outgoing = col.get_outgoing(20);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].target(), 40);
    }

    #[test]
    fn test_delete_removes_node_payload() {
        let (_dir, col) = make_test_collection(None);
        col.upsert_node_payload(10, &serde_json::json!({"k": 1}))
            .unwrap();
        col.upsert_node_payload(20, &serde_json::json!({"k": 2}))
            .unwrap();
        assert_eq!(col.all_node_ids().len(), 2);

        col.delete(&[10]).unwrap();
        assert!(col.get(&[10])[0].is_none(), "deleted node is gone");
        assert!(
            !col.all_node_ids().contains(&10),
            "deleted node leaves the id set"
        );
        assert!(col.get_node_payload(20).unwrap().is_some(), "node 20 stays");
    }

    #[test]
    fn test_scroll_batch_paginates_embedded_nodes() {
        // scroll_batch iterates the point (vector) store, so use embeddings.
        let (_dir, col) = make_test_collection(Some(2));
        for id in [1u64, 2, 3] {
            col.upsert_node(id, &serde_json::json!({"id": id}), Some(vec![1.0, 0.0]))
                .unwrap();
        }
        assert!(!col.is_empty());
        assert_eq!(col.len(), 3);

        let first = col.scroll_batch(None, 2, None).unwrap();
        assert_eq!(first.points.len(), 2, "first page has 2 of 3 nodes");
        let cursor = first.next_cursor.expect("non-empty page yields a cursor");
        let second = col.scroll_batch(Some(cursor), 2, None).unwrap();
        assert_eq!(second.points.len(), 1, "second page has the last node");
        // A page past the end returns no points (and therefore no cursor).
        let tail_cursor = second.next_cursor.expect("page yields a cursor");
        let third = col.scroll_batch(Some(tail_cursor), 2, None).unwrap();
        assert!(third.points.is_empty(), "no points past the end");
        assert!(third.next_cursor.is_none(), "empty page yields no cursor");

        // batch_size 0 is rejected.
        assert!(col.scroll_batch(None, 0, None).is_err());
    }

    #[test]
    fn test_flush_and_flush_full_succeed() {
        let (_dir, col) = make_test_collection(None);
        col.upsert_node_payload(1, &serde_json::json!({"k": 1}))
            .unwrap();
        col.upsert_node_payload(2, &serde_json::json!({})).unwrap();
        col.add_edge(GraphEdge::new(1, 1, 2, "NEXT").unwrap())
            .unwrap();
        col.flush().expect("fast-path flush succeeds");
        col.flush_full().expect("full durability flush succeeds");
    }

    #[test]
    fn test_reopen_recovers_edges_and_payloads() {
        let dir = tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let col = GraphCollection::create(
                path.clone(),
                "kg",
                None,
                DistanceMetric::Cosine,
                GraphSchema::schemaless(),
            )
            .unwrap();
            col.upsert_node_payload(1, &serde_json::json!({"name": "A"}))
                .unwrap();
            col.upsert_node_payload(2, &serde_json::json!({})).unwrap();
            col.add_edge(GraphEdge::new(5, 1, 2, "NEXT").unwrap())
                .unwrap();
            col.flush_full().unwrap();
        }
        let reopened = GraphCollection::open(path).unwrap();
        assert_eq!(reopened.name(), "kg");
        assert!(reopened.has_edge(5), "edge survives reopen");
        assert_eq!(
            reopened.get_node_payload(1).unwrap(),
            Some(serde_json::json!({"name": "A"}))
        );
    }
}
