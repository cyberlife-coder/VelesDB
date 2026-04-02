//! Graph API methods for Collection (EPIC-015 US-001).
//!
//! Exposes Knowledge Graph operations on Collection for use by
//! Tauri plugin, REST API, and other consumers.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::collection::graph::{
    ConcurrentEdgeStore, GraphEdge, GraphSchema, TraversalConfig, TraversalResult,
};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

/// Returns `true` if the edge's label is accepted by the relationship filter.
#[inline]
fn edge_passes_rel_filter(edge: &GraphEdge, rel_types: &[&str]) -> bool {
    rel_types.is_empty() || rel_types.contains(&edge.label())
}

/// Reconstructs the edge-ID path from `source` to `target` using parent pointers.
fn reconstruct_path(parent_map: &FxHashMap<u64, (u64, u64)>, source: u64, target: u64) -> Vec<u64> {
    let mut path = Vec::new();
    let mut current = target;
    while current != source {
        if let Some(&(parent, edge_id)) = parent_map.get(&current) {
            path.push(edge_id);
            current = parent;
        } else {
            break;
        }
    }
    path.reverse();
    path
}

/// Collects unvisited, rel-type-filtered neighbor targets for a node.
///
/// Records parent pointers for lazy path reconstruction (G4).
#[inline]
fn collect_neighbor_expansions(
    edges: &[GraphEdge],
    node: u64,
    depth: u32,
    rel_types: &[&str],
    visited: &mut FxHashSet<u64>,
    parent_map: &mut FxHashMap<u64, (u64, u64)>,
) -> Vec<(u64, u32)> {
    edges
        .iter()
        .filter(|e| edge_passes_rel_filter(e, rel_types))
        .filter(|e| visited.insert(e.target()))
        .map(|e| {
            parent_map.insert(e.target(), (node, e.id()));
            (e.target(), depth + 1)
        })
        .collect()
}

/// Pushes unvisited, rel-type-filtered neighbors onto the DFS stack.
///
/// Records parent pointers for lazy path reconstruction (G4).
#[inline]
fn expand_dfs_neighbors(
    store: &ConcurrentEdgeStore,
    node_id: u64,
    depth: u32,
    rel_filter: &FxHashSet<&str>,
    visited: &FxHashSet<u64>,
    stack: &mut Vec<TraversalEntry>,
    parent_map: &mut FxHashMap<u64, (u64, u64)>,
) {
    let outgoing = store.get_outgoing(node_id);
    for edge in outgoing.iter().rev() {
        if !rel_filter.is_empty() && !rel_filter.contains(edge.label()) {
            continue;
        }
        if visited.contains(&edge.target()) {
            continue;
        }
        parent_map.insert(edge.target(), (node_id, edge.id()));
        stack.push((edge.target(), depth + 1));
    }
}

/// Frontier entry: `(node_id, depth)`. Paths live in the parent-pointer map.
type TraversalEntry = (u64, u32);

/// Bundled parameters for `traverse_with_frontier`.
struct TraversalParams<'a> {
    store: &'a ConcurrentEdgeStore,
    filter: &'a [&'a str],
    limit: usize,
    max_depth: u32,
    source: u64,
}

/// Shared traversal loop for both BFS and DFS.
///
/// Uses parent-pointer map for zero-clone path reconstruction (G4).
fn traverse_with_frontier<F>(
    params: &TraversalParams<'_>,
    pop_fn: fn(&mut F) -> Option<TraversalEntry>,
    push_fn: fn(&mut F, TraversalEntry),
    frontier: &mut F,
) -> Vec<TraversalResult> {
    let mut visited = FxHashSet::default();
    let mut parent_map: FxHashMap<u64, (u64, u64)> = FxHashMap::default();
    let mut results = Vec::new();
    visited.insert(params.source);

    while let Some((node, depth)) = (pop_fn)(frontier) {
        if results.len() >= params.limit {
            break;
        }
        if depth >= params.max_depth {
            continue;
        }

        let outgoing = params.store.get_outgoing(node);
        let neighbors = collect_neighbor_expansions(
            &outgoing,
            node,
            depth,
            params.filter,
            &mut visited,
            &mut parent_map,
        );

        for (target, next_depth) in neighbors {
            let path = reconstruct_path(&parent_map, params.source, target);
            results.push(TraversalResult::new(target, path, next_depth));
            if results.len() >= params.limit {
                break;
            }
            (push_fn)(frontier, (target, next_depth));
        }
    }

    results
}

/// BFS pop: removes from the front of the `VecDeque`.
fn bfs_pop(q: &mut std::collections::VecDeque<TraversalEntry>) -> Option<TraversalEntry> {
    q.pop_front()
}

/// BFS push: appends to the back of the `VecDeque`.
fn bfs_push(q: &mut std::collections::VecDeque<TraversalEntry>, item: TraversalEntry) {
    q.push_back(item);
}

/// DFS pop: removes from the end of the `Vec`.
fn dfs_pop(s: &mut Vec<TraversalEntry>) -> Option<TraversalEntry> {
    s.pop()
}

/// DFS push: appends to the end of the `Vec`.
fn dfs_push(s: &mut Vec<TraversalEntry>, item: TraversalEntry) {
    s.push(item);
}

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
        self.edge_store.add_edge(edge)?;
        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
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
        self.edge_store.all_edges()
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
        self.edge_store.get_edges_by_label(label)
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
        self.edge_store.get_outgoing(node_id)
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
        self.edge_store.get_incoming(node_id)
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
        let filter: &[&str] = rel_types.unwrap_or(&[]);
        let params = TraversalParams {
            store: &self.edge_store,
            filter,
            limit,
            max_depth,
            source,
        };
        let mut frontier = std::collections::VecDeque::new();
        frontier.push_back((source, 0u32));

        Ok(traverse_with_frontier(
            &params,
            bfs_pop,
            bfs_push,
            &mut frontier,
        ))
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
        let filter: &[&str] = rel_types.unwrap_or(&[]);
        let params = TraversalParams {
            store: &self.edge_store,
            filter,
            limit,
            max_depth,
            source,
        };
        let mut frontier = vec![(source, 0u32)];

        Ok(traverse_with_frontier(
            &params,
            dfs_pop,
            dfs_push,
            &mut frontier,
        ))
    }

    /// Gets the in-degree and out-degree of a node.
    ///
    /// Uses degree counters instead of materializing edge vectors for O(1) lookup.
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
        let in_degree = self.edge_store.incoming_degree(node_id);
        let out_degree = self.edge_store.outgoing_degree(node_id);
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
        // Atomic check-and-remove — no TOCTOU race.
        let removed = self.edge_store.remove_edge(edge_id);
        if removed {
            self.write_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        removed
    }

    /// Returns the total number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edge_store.len()
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
    /// Also maintains the label index: if the payload contains a `_labels`
    /// array, each label is indexed for O(1) lookup in `find_start_nodes()`.
    /// On update (existing node), old labels are removed before new ones
    /// are inserted.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn store_node_payload(&self, node_id: u64, payload: &serde_json::Value) -> Result<()> {
        // LOCK ORDER: payload_storage(3) → label_index(7).
        let mut storage = self.payload_storage.write();

        // Remove old labels if this is an update (not a fresh insert).
        let mut label_idx = self.label_index.write();
        if let Ok(Some(old_payload)) = storage.retrieve(node_id) {
            label_idx.remove_from_payload(node_id, &old_payload);
        }

        storage.store(node_id, payload)?;
        label_idx.index_from_payload(node_id, payload);

        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Retrieves the JSON payload for a graph node.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval fails.
    pub fn get_node_payload(&self, node_id: u64) -> Result<Option<serde_json::Value>> {
        Ok(self.payload_storage.read().retrieve(node_id)?)
    }

    // -------------------------------------------------------------------------
    // Graph traversal with TraversalConfig
    // -------------------------------------------------------------------------

    /// BFS traversal using the core `concurrent_bfs_stream` iterator.
    #[must_use]
    pub fn traverse_bfs_config(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        use crate::collection::graph::{concurrent_bfs_stream, StreamingConfig};
        let streaming = StreamingConfig {
            max_depth: config.max_depth,
            rel_types: config.rel_types.clone(),
            limit: Some(config.limit),
            max_visited_size: 100_000,
        };
        concurrent_bfs_stream(&self.edge_store, source_id, streaming)
            .filter(|result| result.depth >= config.min_depth)
            .take(config.limit)
            .collect()
    }

    /// DFS traversal (iterative) using `TraversalConfig`.
    ///
    /// Uses parent-pointer map for zero-clone path reconstruction (G4).
    #[must_use]
    pub fn traverse_dfs_config(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        let rel_filter: FxHashSet<&str> = config.rel_types.iter().map(String::as_str).collect();

        let mut results = Vec::new();
        let mut visited: FxHashSet<u64> = FxHashSet::default();
        let mut parent_map: FxHashMap<u64, (u64, u64)> = FxHashMap::default();
        let mut stack: Vec<TraversalEntry> = vec![(source_id, 0)];

        while let Some((node_id, depth)) = stack.pop() {
            if results.len() >= config.limit {
                break;
            }
            if !visited.insert(node_id) {
                continue;
            }
            if depth >= config.min_depth && depth > 0 {
                let path = reconstruct_path(&parent_map, source_id, node_id);
                results.push(TraversalResult::new(node_id, path, depth));
                if results.len() >= config.limit {
                    break;
                }
            }
            if depth < config.max_depth {
                expand_dfs_neighbors(
                    &self.edge_store,
                    node_id,
                    depth,
                    &rel_filter,
                    &visited,
                    &mut stack,
                    &mut parent_map,
                );
            }
        }
        results
    }

    /// Parallel BFS traversal from multiple start nodes using rayon.
    ///
    /// When `start_nodes` exceeds the parallel threshold (100), rayon distributes
    /// independent per-start-node BFS traversals across CPU cores. Below the
    /// threshold, falls back to sequential execution.
    ///
    /// Results are deduplicated by path signature and truncated to `config.limit`.
    #[must_use]
    pub fn traverse_bfs_parallel(
        &self,
        start_nodes: &[u64],
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        use crate::collection::search::query::parallel_traversal::{
            ParallelConfig, ParallelTraverser,
        };

        let par_config = ParallelConfig::new()
            .with_max_depth(config.max_depth)
            .with_limit(config.limit);

        let traverser = ParallelTraverser::with_config(par_config);
        let edge_store = &self.edge_store;

        let rel_types = &config.rel_types;
        let adjacency = |node: u64| -> Vec<(u64, u64)> {
            edge_store
                .get_outgoing(node)
                .into_iter()
                .filter(|e| rel_types.is_empty() || rel_types.contains(&e.label().to_string()))
                .map(|e| (e.target(), e.id()))
                .collect()
        };

        let (results, _stats) = traverser.bfs_parallel(start_nodes, adjacency);

        results
            .into_iter()
            .filter(|r| r.depth >= config.min_depth)
            .map(|r| TraversalResult::new(r.end_node, r.path, r.depth))
            .collect()
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
        let metric = self.config.read().metric;
        let ids = self.index.search(query, k);
        let ids = self.merge_delta(ids, query, k, metric);

        // Acquire each lock once: collect vector data, then collect payload data.
        // This avoids holding vector_storage while locking payload_storage per item.
        let vectors: Vec<(u64, f32, Option<Vec<f32>>)> = {
            let vector_storage = self.vector_storage.read();
            ids.into_iter()
                .map(|sr| {
                    let vec = vector_storage.retrieve(sr.id).ok().flatten();
                    (sr.id, sr.score, vec)
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
                            sparse_vectors: None,
                        },
                        score,
                    ))
                })
                .collect()
        };
        Ok(results)
    }
}
