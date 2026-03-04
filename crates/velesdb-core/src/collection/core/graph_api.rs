//! Graph API methods for Collection (EPIC-015 US-001).
//!
//! Exposes Knowledge Graph operations on Collection for use by
//! Tauri plugin, REST API, and other consumers.

use crate::collection::graph::{GraphEdge, GraphSchema, TraversalConfig, TraversalResult};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Adds an edge to the collection's knowledge graph.
    ///
    /// # Arguments
    ///
    /// * `edge` - The edge to add (id, source, target, label, properties)
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::collection::graph::GraphEdge;
    ///
    /// let edge = GraphEdge::new(1, 100, 200, "KNOWS")?;
    /// collection.add_edge(edge)?;
    /// ```
    pub fn add_edge(&self, edge: GraphEdge) -> Result<()> {
        self.edge_store.write().add_edge(edge)
    }

    /// Gets all edges from the collection's knowledge graph.
    ///
    /// Note: This iterates through all stored edges. For large graphs,
    /// consider using `get_edges_by_label` or `get_outgoing_edges` for
    /// more targeted queries.
    ///
    /// # Returns
    ///
    /// Vector of all edges in the graph (cloned).
    #[must_use]
    pub fn get_all_edges(&self) -> Vec<GraphEdge> {
        let store = self.edge_store.read();
        store.all_edges().into_iter().cloned().collect()
    }

    /// Gets edges filtered by label.
    ///
    /// # Arguments
    ///
    /// * `label` - The edge label (relationship type) to filter by
    ///
    /// # Returns
    ///
    /// Vector of edges with the specified label (cloned).
    #[must_use]
    pub fn get_edges_by_label(&self, label: &str) -> Vec<GraphEdge> {
        self.edge_store
            .read()
            .get_edges_by_label(label)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Gets outgoing edges from a specific node.
    ///
    /// # Arguments
    ///
    /// * `node_id` - The source node ID
    ///
    /// # Returns
    ///
    /// Vector of edges originating from the specified node (cloned).
    #[must_use]
    pub fn get_outgoing_edges(&self, node_id: u64) -> Vec<GraphEdge> {
        self.edge_store
            .read()
            .get_outgoing(node_id)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Gets incoming edges to a specific node.
    ///
    /// # Arguments
    ///
    /// * `node_id` - The target node ID
    ///
    /// # Returns
    ///
    /// Vector of edges pointing to the specified node (cloned).
    #[must_use]
    pub fn get_incoming_edges(&self, node_id: u64) -> Vec<GraphEdge> {
        self.edge_store
            .read()
            .get_incoming(node_id)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Traverses the graph using BFS from a source node.
    ///
    /// # Arguments
    ///
    /// * `source` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `rel_types` - Optional filter by relationship types
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Vector of traversal results with target nodes and paths.
    ///
    /// # Errors
    ///
    /// Returns an error if traversal fails.
    pub fn traverse_bfs(
        &self,
        source: u64,
        max_depth: u32,
        rel_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<TraversalResult>> {
        use std::collections::{HashSet, VecDeque};

        let store = self.edge_store.read();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut results = Vec::new();

        visited.insert(source);
        queue.push_back((source, 0u32, Vec::new()));

        while let Some((node, depth, path)) = queue.pop_front() {
            if results.len() >= limit {
                break;
            }

            if depth >= max_depth {
                continue;
            }

            for edge in store.get_outgoing(node) {
                // Filter by relationship type if specified
                if let Some(types) = rel_types {
                    if !types.contains(&edge.label()) {
                        continue;
                    }
                }

                let target = edge.target();
                if !visited.contains(&target) {
                    visited.insert(target);
                    let mut new_path = path.clone();
                    new_path.push(edge.id());

                    results.push(TraversalResult {
                        target_id: target,
                        depth: depth + 1,
                        path: new_path.clone(),
                    });

                    if results.len() < limit {
                        queue.push_back((target, depth + 1, new_path));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Traverses the graph using DFS from a source node.
    ///
    /// # Arguments
    ///
    /// * `source` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `rel_types` - Optional filter by relationship types
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Vector of traversal results with target nodes and paths.
    ///
    /// # Errors
    ///
    /// Returns an error if traversal fails.
    pub fn traverse_dfs(
        &self,
        source: u64,
        max_depth: u32,
        rel_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<TraversalResult>> {
        use std::collections::HashSet;

        let store = self.edge_store.read();
        let mut visited = HashSet::new();
        let mut stack = Vec::new();
        let mut results = Vec::new();

        visited.insert(source);
        stack.push((source, 0u32, Vec::new()));

        while let Some((node, depth, path)) = stack.pop() {
            if results.len() >= limit {
                break;
            }

            if depth >= max_depth {
                continue;
            }

            for edge in store.get_outgoing(node) {
                // Filter by relationship type if specified
                if let Some(types) = rel_types {
                    if !types.contains(&edge.label()) {
                        continue;
                    }
                }

                let target = edge.target();
                if !visited.contains(&target) {
                    visited.insert(target);
                    let mut new_path = path.clone();
                    new_path.push(edge.id());

                    results.push(TraversalResult {
                        target_id: target,
                        depth: depth + 1,
                        path: new_path.clone(),
                    });

                    if results.len() < limit {
                        stack.push((target, depth + 1, new_path));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Gets the in-degree and out-degree of a node.
    ///
    /// # Arguments
    ///
    /// * `node_id` - The node ID
    ///
    /// # Returns
    ///
    /// Tuple of (`in_degree`, `out_degree`).
    #[must_use]
    pub fn get_node_degree(&self, node_id: u64) -> (usize, usize) {
        let store = self.edge_store.read();
        let in_degree = store.get_incoming(node_id).len();
        let out_degree = store.get_outgoing(node_id).len();
        (in_degree, out_degree)
    }

    /// Removes an edge from the graph by ID.
    ///
    /// # Arguments
    ///
    /// * `edge_id` - The edge ID to remove
    ///
    /// # Returns
    ///
    /// `true` if the edge existed and was removed, `false` if it didn't exist.
    #[must_use]
    pub fn remove_edge(&self, edge_id: u64) -> bool {
        let mut store = self.edge_store.write();
        if store.contains_edge(edge_id) {
            store.remove_edge(edge_id);
            true
        } else {
            false
        }
    }

    /// Returns the total number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edge_store.read().len()
    }

    // -------------------------------------------------------------------------
    // Graph schema
    // -------------------------------------------------------------------------

    /// Returns the graph schema stored in the collection config, if any.
    #[must_use]
    pub fn graph_schema(&self) -> Option<GraphSchema> {
        self.config.read().graph_schema.clone()
    }

    /// Returns `true` if this collection was created as a graph collection.
    #[must_use]
    pub fn is_graph(&self) -> bool {
        self.config.read().graph_schema.is_some()
    }

    /// Returns `true` if this graph collection stores node embeddings.
    #[must_use]
    pub fn has_embeddings(&self) -> bool {
        self.config.read().embedding_dimension.is_some()
    }

    // -------------------------------------------------------------------------
    // Node payload (graph node properties)
    // -------------------------------------------------------------------------

    /// Stores a JSON payload for a graph node.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn store_node_payload(&self, node_id: u64, payload: &serde_json::Value) -> Result<()> {
        let mut storage = self.payload_storage.write();
        storage.store(node_id, payload).map_err(Error::Io)
    }

    /// Retrieves the JSON payload for a graph node.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval fails.
    pub fn get_node_payload(&self, node_id: u64) -> Result<Option<serde_json::Value>> {
        self.payload_storage
            .read()
            .retrieve(node_id)
            .map_err(Error::Io)
    }

    // -------------------------------------------------------------------------
    // Graph traversal with TraversalConfig
    // -------------------------------------------------------------------------

    /// BFS traversal using the core `bfs_stream` iterator.
    #[must_use]
    pub fn traverse_bfs_config(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        use crate::collection::graph::{bfs_stream, StreamingConfig};
        let store = self.edge_store.read();
        let streaming = StreamingConfig {
            max_depth: config.max_depth,
            rel_types: config.rel_types.clone(),
            limit: Some(config.limit),
            max_visited_size: 100_000,
        };
        bfs_stream(&store, source_id, streaming)
            .filter(|result| result.depth >= config.min_depth)
            .take(config.limit)
            .collect()
    }

    /// DFS traversal (iterative) using `TraversalConfig`.
    #[must_use]
    pub fn traverse_dfs_config(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        use std::collections::HashSet;
        let store = self.edge_store.read();
        let rel_filter: HashSet<&str> = config.rel_types.iter().map(String::as_str).collect();

        let mut results = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut stack: Vec<(u64, u32, Vec<u64>)> = vec![(source_id, 0, Vec::new())];

        while let Some((node_id, depth, path)) = stack.pop() {
            if results.len() >= config.limit {
                break;
            }
            if !visited.insert(node_id) {
                continue;
            }
            if depth >= config.min_depth && depth > 0 {
                results.push(TraversalResult::new(node_id, path.clone(), depth));
                if results.len() >= config.limit {
                    break;
                }
            }
            if depth < config.max_depth {
                for edge in store.get_outgoing(node_id).into_iter().rev() {
                    if !rel_filter.is_empty() && !rel_filter.contains(edge.label()) {
                        continue;
                    }
                    if visited.contains(&edge.target()) {
                        continue;
                    }
                    let mut new_path = path.clone();
                    // Use edge IDs in path, consistent with bfs_traverse/bfs_stream.
                    new_path.push(edge.id());
                    stack.push((edge.target(), depth + 1, new_path));
                }
            }
        }
        results
    }

    // -------------------------------------------------------------------------
    // Embedding search on graph nodes
    // -------------------------------------------------------------------------

    /// Searches for similar graph nodes by embedding vector.
    ///
    /// Only available if `has_embeddings()` returns `true`.
    ///
    /// # Errors
    ///
    /// Returns `Error::VectorNotAllowed` if no embeddings are configured,
    /// or `Error::DimensionMismatch` if the query dimension is wrong.
    pub fn search_by_embedding(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let config = self.config.read();
        let emb_dim = config
            .embedding_dimension
            .ok_or_else(|| Error::VectorNotAllowed(config.name.clone()))?;
        drop(config);

        if query.len() != emb_dim {
            return Err(Error::DimensionMismatch {
                expected: emb_dim,
                actual: query.len(),
            });
        }

        // Reason: we reuse the existing HNSW index (dimension == emb_dim when created
        // via create_graph_collection_with_embeddings). For graph-without-embeddings
        // the HNSW has dimension 0 and the guard above already rejected the call.
        let ids = self.index.search(query, k);

        // BUG-4: acquire each lock once for all results instead of holding
        // vector_storage while repeatedly locking payload_storage per item.
        let vectors: Vec<(u64, f32, Option<Vec<f32>>)> = {
            let vector_storage = self.vector_storage.read();
            ids.into_iter()
                .map(|(id, score)| {
                    let vec = vector_storage.retrieve(id).ok().flatten();
                    (id, score, vec)
                })
                .collect()
        };
        let results = {
            let payload_storage = self.payload_storage.read();
            vectors
                .into_iter()
                .filter_map(|(id, score, vector)| {
                    let vector = vector?;
                    let payload = payload_storage.retrieve(id).ok().flatten();
                    Some(SearchResult::new(
                        Point {
                            id,
                            vector,
                            payload,
                        },
                        score,
                    ))
                })
                .collect()
        };
        Ok(results)
    }
}
