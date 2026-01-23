//! Graph service for VelesDB REST API.
//!
//! Manages per-collection edge stores for graph operations.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use velesdb_core::collection::graph::{EdgeStore, GraphEdge};

use super::types::TraversalResultItem;

/// Shared graph service state for managing per-collection edge stores.
#[derive(Clone, Default)]
pub struct GraphService {
    stores: Arc<RwLock<HashMap<String, Arc<RwLock<EdgeStore>>>>>,
}

impl GraphService {
    /// Creates a new graph service.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Gets or creates an edge store for a collection.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    pub fn get_or_create_store(
        &self,
        collection_name: &str,
    ) -> Result<Arc<RwLock<EdgeStore>>, String> {
        let mut stores = self
            .stores
            .write()
            .map_err(|e| format!("Lock poisoned: {e}"))?;
        Ok(stores
            .entry(collection_name.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(EdgeStore::new())))
            .clone())
    }

    /// Adds an edge to a collection's graph.
    ///
    /// # Errors
    ///
    /// Returns an error if the lock is poisoned or if adding the edge fails.
    pub fn add_edge(&self, collection_name: &str, edge: GraphEdge) -> Result<(), String> {
        let store = self.get_or_create_store(collection_name)?;
        let mut guard = store.write().map_err(|e| format!("Lock error: {e}"))?;
        guard.add_edge(edge).map_err(|e| e.to_string())
    }

    /// Gets edges by label from a collection's graph.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    pub fn get_edges_by_label(
        &self,
        collection_name: &str,
        label: &str,
    ) -> Result<Vec<GraphEdge>, String> {
        let store = self.get_or_create_store(collection_name)?;
        let guard = store.read().map_err(|e| format!("Lock poisoned: {e}"))?;
        Ok(guard
            .get_edges_by_label(label)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Lists all stores (for metrics).
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    #[allow(clippy::type_complexity)]
    pub fn list_stores(&self) -> Result<Vec<(String, Arc<RwLock<EdgeStore>>)>, String> {
        let stores = self
            .stores
            .read()
            .map_err(|e| format!("Lock poisoned: {e}"))?;
        Ok(stores.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    /// Performs BFS traversal from a source node.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    pub fn traverse_bfs(
        &self,
        collection_name: &str,
        source_id: u64,
        max_depth: u32,
        limit: usize,
        rel_types: &[String],
    ) -> Result<Vec<TraversalResultItem>, String> {
        let store = self.get_or_create_store(collection_name)?;
        let guard = store.read().map_err(|e| format!("Lock error: {e}"))?;

        // PERF: Convert rel_types to HashSet for O(1) lookup instead of O(k)
        let rel_filter: HashSet<&str> = rel_types.iter().map(String::as_str).collect();

        let mut results = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut queue: VecDeque<(u64, u32, Vec<u64>)> = VecDeque::new();

        visited.insert(source_id);
        queue.push_back((source_id, 0, Vec::new()));

        while let Some((node_id, depth, path)) = queue.pop_front() {
            if results.len() >= limit {
                break;
            }

            let edges = guard.get_outgoing(node_id);
            for edge in edges {
                if !rel_filter.is_empty() && !rel_filter.contains(edge.label()) {
                    continue;
                }

                let target = edge.target();
                let new_depth = depth + 1;

                if new_depth > max_depth || visited.contains(&target) {
                    continue;
                }
                visited.insert(target);

                let mut new_path = path.clone();
                new_path.push(edge.id());

                results.push(TraversalResultItem {
                    target_id: target,
                    depth: new_depth,
                    path: new_path.clone(),
                });

                if results.len() >= limit {
                    break;
                }

                if new_depth < max_depth {
                    queue.push_back((target, new_depth, new_path));
                }
            }
        }

        Ok(results)
    }

    /// Performs DFS traversal from a source node.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    pub fn traverse_dfs(
        &self,
        collection_name: &str,
        source_id: u64,
        max_depth: u32,
        limit: usize,
        rel_types: &[String],
    ) -> Result<Vec<TraversalResultItem>, String> {
        let store = self.get_or_create_store(collection_name)?;
        let guard = store.read().map_err(|e| format!("Lock error: {e}"))?;

        // PERF: Convert rel_types to HashSet for O(1) lookup instead of O(k)
        let rel_filter: HashSet<&str> = rel_types.iter().map(String::as_str).collect();

        let mut results = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut stack: Vec<(u64, u32, Vec<u64>)> = vec![(source_id, 0, Vec::new())];

        while let Some((node_id, depth, path)) = stack.pop() {
            if results.len() >= limit {
                break;
            }

            if visited.contains(&node_id) {
                continue;
            }
            visited.insert(node_id);

            if depth > 0 {
                results.push(TraversalResultItem {
                    target_id: node_id,
                    depth,
                    path: path.clone(),
                });

                if results.len() >= limit {
                    break;
                }
            }

            if depth < max_depth {
                let edges = guard.get_outgoing(node_id);
                let filtered: Vec<_> = edges
                    .into_iter()
                    .filter(|e| rel_filter.is_empty() || rel_filter.contains(e.label()))
                    .filter(|e| !visited.contains(&e.target()))
                    .collect();

                for edge in filtered.into_iter().rev() {
                    let mut new_path = path.clone();
                    new_path.push(edge.id());
                    stack.push((edge.target(), depth + 1, new_path));
                }
            }
        }

        Ok(results)
    }

    /// Gets the in-degree and out-degree of a node.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    pub fn get_node_degree(
        &self,
        collection_name: &str,
        node_id: u64,
    ) -> Result<(usize, usize), String> {
        let store = self.get_or_create_store(collection_name)?;
        let guard = store.read().map_err(|e| format!("Lock error: {e}"))?;

        let in_degree = guard.get_incoming(node_id).len();
        let out_degree = guard.get_outgoing(node_id).len();

        Ok((in_degree, out_degree))
    }
}
