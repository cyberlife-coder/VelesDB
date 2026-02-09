//! Graph traversal algorithms (BFS/DFS) for in-memory graphs.
//!
//! Provides generic traversal via the [`GraphTraversal`] trait, enabling
//! any graph store to support BFS and DFS without reimplementation.

use std::collections::{HashSet, VecDeque};

/// Trait for graph traversal — any graph store can implement this.
///
/// Returns outgoing edges as `(edge_id, target_node_id, label)` triples.
pub trait GraphTraversal {
    /// Returns outgoing edges from a node as `(edge_id, target, label)`.
    fn outgoing_edges(&self, node_id: u64) -> Vec<(u64, u64, String)>;

    /// Returns incoming edges to a node as `(edge_id, source, label)`.
    fn incoming_edges(&self, node_id: u64) -> Vec<(u64, u64, String)>;
}

/// Implement `GraphTraversal` for `InMemoryEdgeStore`.
impl GraphTraversal for super::InMemoryEdgeStore {
    fn outgoing_edges(&self, node_id: u64) -> Vec<(u64, u64, String)> {
        self.get_outgoing(node_id)
            .into_iter()
            .map(|e| (e.id(), e.target(), e.label().to_string()))
            .collect()
    }

    fn incoming_edges(&self, node_id: u64) -> Vec<(u64, u64, String)> {
        self.get_incoming(node_id)
            .into_iter()
            .map(|e| (e.id(), e.source(), e.label().to_string()))
            .collect()
    }
}

/// A single step in a graph traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraversalStep {
    /// The node ID reached at this step.
    pub node_id: u64,
    /// Depth of this step (number of hops from source).
    pub depth: usize,
    /// Path taken to reach this node (list of edge IDs).
    pub path: Vec<u64>,
}

/// Configuration for graph traversal.
#[derive(Debug, Clone)]
pub struct TraversalConfig {
    /// Maximum traversal depth.
    pub max_depth: usize,
    /// Maximum number of results.
    pub limit: usize,
    /// Filter by relationship types (empty = all types).
    pub rel_types: Vec<String>,
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            limit: 100,
            rel_types: Vec::new(),
        }
    }
}

impl TraversalConfig {
    /// Creates a config with the given max depth and limit.
    #[must_use]
    pub fn new(max_depth: usize, limit: usize) -> Self {
        Self {
            max_depth,
            limit,
            rel_types: Vec::new(),
        }
    }

    /// Sets relationship type filter (builder pattern).
    #[must_use]
    pub fn with_rel_types(mut self, types: Vec<String>) -> Self {
        self.rel_types = types;
        self
    }
}

/// BFS traversal from a source node.
///
/// Finds all reachable nodes within `config.max_depth` hops, returning
/// up to `config.limit` results. Optionally filters by relationship type.
///
/// # Arguments
///
/// * `graph` - Any graph implementing `GraphTraversal`
/// * `source_id` - Starting node ID
/// * `config` - Traversal configuration
#[must_use]
pub fn bfs<G: GraphTraversal>(
    graph: &G,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalStep> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    visited.insert(source_id);
    queue.push_back((source_id, 0usize, Vec::<u64>::new()));

    while let Some((current, depth, path)) = queue.pop_front() {
        if results.len() >= config.limit {
            break;
        }

        for (edge_id, target, label) in graph.outgoing_edges(current) {
            if !config.rel_types.is_empty() && !config.rel_types.contains(&label) {
                continue;
            }

            let new_depth = depth + 1;
            if new_depth > config.max_depth {
                continue;
            }

            let mut new_path = path.clone();
            new_path.push(edge_id);

            results.push(TraversalStep {
                node_id: target,
                depth: new_depth,
                path: new_path.clone(),
            });

            if results.len() >= config.limit {
                break;
            }

            if new_depth < config.max_depth && !visited.contains(&target) {
                visited.insert(target);
                queue.push_back((target, new_depth, new_path));
            }
        }
    }

    results
}

/// DFS traversal from a source node.
///
/// Finds all reachable nodes within `config.max_depth` hops using
/// depth-first order, returning up to `config.limit` results.
#[must_use]
pub fn dfs<G: GraphTraversal>(
    graph: &G,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalStep> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();

    visited.insert(source_id);
    dfs_recursive(graph, source_id, 0, &[], &mut visited, &mut results, config);

    results
}

fn dfs_recursive<G: GraphTraversal>(
    graph: &G,
    current: u64,
    depth: usize,
    path: &[u64],
    visited: &mut HashSet<u64>,
    results: &mut Vec<TraversalStep>,
    config: &TraversalConfig,
) {
    if results.len() >= config.limit || depth >= config.max_depth {
        return;
    }

    for (edge_id, target, label) in graph.outgoing_edges(current) {
        if results.len() >= config.limit {
            break;
        }

        if !config.rel_types.is_empty() && !config.rel_types.contains(&label) {
            continue;
        }

        let mut new_path = path.to_owned();
        new_path.push(edge_id);

        results.push(TraversalStep {
            node_id: target,
            depth: depth + 1,
            path: new_path.clone(),
        });

        if !visited.contains(&target) {
            visited.insert(target);
            dfs_recursive(
                graph,
                target,
                depth + 1,
                &new_path,
                visited,
                results,
                config,
            );
        }
    }
}
