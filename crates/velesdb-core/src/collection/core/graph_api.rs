//! Graph API methods for Collection (EPIC-015 US-001).
//!
//! Exposes Knowledge Graph operations on Collection for use by
//! Tauri plugin, REST API, and other consumers.

use crate::collection::graph::GraphEdge;
use crate::collection::types::Collection;
use crate::error::Result;

/// Traversal result for graph operations.
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// Target node ID reached.
    pub target_id: u64,
    /// Depth of traversal.
    pub depth: u32,
    /// Path taken (node IDs).
    pub path: Vec<u64>,
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
        queue.push_back((source, 0u32, vec![source]));

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
                    new_path.push(target);

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
        stack.push((source, 0u32, vec![source]));

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
                    new_path.push(target);

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
    /// Tuple of (in_degree, out_degree).
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
        let store = self.edge_store.read();
        store.len()
    }
}
