//! Graph traversal algorithms for multi-hop queries.
//!
//! This module provides BFS-based traversal for variable-length path patterns
//! like `(a)-[*1..3]->(b)` in MATCH clauses.
//!
//! # Streaming Mode (EPIC-019 US-005)
//!
//! For large graphs, the module provides streaming iterators that yield results
//! lazily without loading all visited nodes into memory at once.

#![allow(dead_code)] // WIP: Will be used by MATCH clause execution

use super::EdgeStore;
use std::collections::{HashSet, VecDeque};

/// Default maximum depth for unbounded traversals.
pub const DEFAULT_MAX_DEPTH: u32 = 3;

/// Safety cap for maximum depth to prevent runaway traversals.
/// Only applied when user requests unbounded traversal (*).
///
/// Note: Neo4j and ArangoDB do NOT impose hard limits.
/// 100 is chosen to cover most real-world use cases:
/// - Social networks (6 degrees of separation)
/// - Dependency graphs (deep npm/cargo trees)
/// - Organizational hierarchies
/// - Knowledge graphs
pub const SAFETY_MAX_DEPTH: u32 = 100;

/// Result of a graph traversal operation.
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// The target node ID reached.
    pub target_id: u64,
    /// The path taken (list of edge IDs).
    pub path: Vec<u64>,
    /// Depth of the traversal (number of hops).
    pub depth: u32,
}

impl TraversalResult {
    /// Creates a new traversal result.
    #[must_use]
    pub fn new(target_id: u64, path: Vec<u64>, depth: u32) -> Self {
        Self {
            target_id,
            path,
            depth,
        }
    }
}

/// Configuration for graph traversal.
#[derive(Debug, Clone)]
pub struct TraversalConfig {
    /// Minimum number of hops (inclusive).
    pub min_depth: u32,
    /// Maximum number of hops (inclusive).
    pub max_depth: u32,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Filter by relationship types (empty = all types).
    pub rel_types: Vec<String>,
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            min_depth: 1,
            max_depth: DEFAULT_MAX_DEPTH,
            limit: 100,
            rel_types: Vec::new(),
        }
    }
}

impl TraversalConfig {
    /// Creates a config for a specific range (e.g., *1..3).
    ///
    /// Respects the caller's max_depth without artificial capping.
    /// For unbounded traversals, use `with_unbounded_range()` instead.
    #[must_use]
    pub fn with_range(min: u32, max: u32) -> Self {
        Self {
            min_depth: min,
            max_depth: max,
            ..Self::default()
        }
    }

    /// Creates a config for unbounded traversal (e.g., *1..).
    ///
    /// Applies SAFETY_MAX_DEPTH cap to prevent runaway traversals.
    #[must_use]
    pub fn with_unbounded_range(min: u32) -> Self {
        Self {
            min_depth: min,
            max_depth: SAFETY_MAX_DEPTH,
            ..Self::default()
        }
    }

    /// Sets the result limit.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Filters by relationship types.
    #[must_use]
    pub fn with_rel_types(mut self, types: Vec<String>) -> Self {
        self.rel_types = types;
        self
    }

    /// Sets a custom max depth (for advanced use cases).
    #[must_use]
    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }
}

/// BFS state for traversal.
#[derive(Debug)]
pub(super) struct BfsState {
    /// Current node ID.
    pub(super) node_id: u64,
    /// Path taken to reach this node (edge IDs).
    pub(super) path: Vec<u64>,
    /// Current depth.
    pub(super) depth: u32,
}

/// Performs BFS traversal from a source node.
///
/// Finds all paths from `source_id` within the configured depth range.
/// Uses iterative BFS with `VecDeque` for better cache locality.
///
/// # Arguments
///
/// * `edge_store` - The edge storage to traverse.
/// * `source_id` - Starting node ID.
/// * `config` - Traversal configuration.
///
/// # Returns
///
/// Vector of traversal results, limited by `config.limit`.
#[must_use]
pub fn bfs_traverse(
    edge_store: &EdgeStore,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // CRITICAL FIX: Mark source node as visited before traversal
    // to prevent cycles back to source causing duplicate work
    visited.insert(source_id);

    // Initialize with source node
    queue.push_back(BfsState {
        node_id: source_id,
        path: Vec::new(),
        depth: 0,
    });

    while let Some(state) = queue.pop_front() {
        // Check if we've collected enough results
        if results.len() >= config.limit {
            break;
        }

        // Get outgoing edges from current node
        let edges = edge_store.get_outgoing(state.node_id);

        for edge in edges {
            // Filter by relationship type if specified
            if !config.rel_types.is_empty() && !config.rel_types.contains(&edge.label().to_string())
            {
                continue;
            }

            let target = edge.target();
            let new_depth = state.depth + 1;

            // Skip if exceeds max depth
            if new_depth > config.max_depth {
                continue;
            }

            // Build new path
            let mut new_path = state.path.clone();
            new_path.push(edge.id());

            // Add to results if within valid depth range
            if new_depth >= config.min_depth {
                results.push(TraversalResult::new(target, new_path.clone(), new_depth));

                if results.len() >= config.limit {
                    break;
                }
            }

            // Continue traversal if not at max depth and not visited
            if new_depth < config.max_depth && !visited.contains(&target) {
                visited.insert(target);
                queue.push_back(BfsState {
                    node_id: target,
                    path: new_path,
                    depth: new_depth,
                });
            }
        }
    }

    results
}

/// Performs BFS traversal in the reverse direction (following incoming edges).
#[must_use]
pub fn bfs_traverse_reverse(
    edge_store: &EdgeStore,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // CRITICAL FIX: Mark source node as visited before traversal
    visited.insert(source_id);

    queue.push_back(BfsState {
        node_id: source_id,
        path: Vec::new(),
        depth: 0,
    });

    while let Some(state) = queue.pop_front() {
        if results.len() >= config.limit {
            break;
        }

        let edges = edge_store.get_incoming(state.node_id);

        for edge in edges {
            if !config.rel_types.is_empty() {
                let label = edge.label().to_string();
                if !config.rel_types.contains(&label) {
                    continue;
                }
            }

            let source = edge.source();
            let new_depth = state.depth + 1;

            if new_depth > config.max_depth {
                continue;
            }

            let mut new_path = state.path.clone();
            new_path.push(edge.id());

            if new_depth >= config.min_depth {
                results.push(TraversalResult::new(source, new_path.clone(), new_depth));

                if results.len() >= config.limit {
                    break;
                }
            }

            if new_depth < config.max_depth && !visited.contains(&source) {
                visited.insert(source);
                queue.push_back(BfsState {
                    node_id: source,
                    path: new_path,
                    depth: new_depth,
                });
            }
        }
    }

    results
}

/// Performs bidirectional BFS (follows both directions).
#[must_use]
pub fn bfs_traverse_both(
    edge_store: &EdgeStore,
    source_id: u64,
    config: &TraversalConfig,
) -> Vec<TraversalResult> {
    let mut results = Vec::new();
    let half_limit = config.limit / 2 + 1;

    let config_half = TraversalConfig {
        limit: half_limit,
        ..config.clone()
    };

    // Forward traversal
    let forward = bfs_traverse(edge_store, source_id, &config_half);
    results.extend(forward);

    // Reverse traversal
    if results.len() < config.limit {
        let reverse = bfs_traverse_reverse(edge_store, source_id, &config_half);
        for r in reverse {
            if results.len() >= config.limit {
                break;
            }
            // Avoid duplicates
            if !results
                .iter()
                .any(|existing| existing.target_id == r.target_id && existing.path == r.path)
            {
                results.push(r);
            }
        }
    }

    results.truncate(config.limit);
    results
}

// Tests moved to traversal_tests.rs per project rules
