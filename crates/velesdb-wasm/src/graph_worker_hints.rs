//! Heuristic helpers for Web Worker offload decisions (EPIC-053/US-005).
//!
//! This module provides client-side heuristics only. It does **not** create,
//! instantiate, or orchestrate Web Workers. No `postMessage` channel, no
//! worker lifecycle, no `SharedArrayBuffer` transfer is implemented here.
//!
//! Callers that wish to offload traversal to a Web Worker are responsible
//! for instantiating a JS `Worker`, serializing graph data, and dispatching
//! messages. This module only exposes:
//!
//! - [`GraphWorkerConfig`] — configurable thresholds for offload decisions.
//! - [`should_use_worker`] — threshold check based on node count and depth.
//! - [`estimate_traversal_size`] — BFS frontier heuristic for progress UIs.
//! - [`TraversalProgress`] — data class for progress reporting by callers.
//!
//! # Usage (JavaScript)
//!
//! ```javascript
//! import { GraphWorkerConfig, should_use_worker } from 'velesdb-wasm';
//!
//! // Heuristic check only. Caller decides what to do with the result.
//! if (should_use_worker(graphStore.node_count, maxDepth)) {
//!   // Application code must create and manage the worker itself.
//!   const worker = new Worker('./velesdb-graph-worker.js');
//!   worker.postMessage({ type: 'traverse', ... });
//! } else {
//!   // Synchronous traversal on main thread.
//!   const results = graphStore.bfs_traverse(startNode, maxDepth, limit);
//! }
//! ```

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// Thresholds used by [`should_use_worker`] to recommend Web Worker offload.
///
/// This is a data-only configuration object. It does not allocate or start
/// a Worker — the caller remains responsible for Worker lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[wasm_bindgen]
#[allow(clippy::unsafe_derive_deserialize)]
pub struct GraphWorkerConfig {
    /// Minimum node count at which offload is recommended.
    pub node_threshold: usize,
    /// Minimum traversal depth at which offload is recommended.
    pub depth_threshold: usize,
    /// Suggested progress callback interval in milliseconds for the
    /// caller's own worker implementation. This crate does not emit
    /// progress events on its own.
    pub progress_interval_ms: u32,
}

#[wasm_bindgen]
impl GraphWorkerConfig {
    /// Creates a new configuration with default values.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns thresholds tuned for large graphs.
    #[wasm_bindgen]
    pub fn for_large_graphs() -> Self {
        Self {
            node_threshold: 10_000,
            depth_threshold: 5,
            progress_interval_ms: 50,
        }
    }

    /// Returns thresholds tuned for UI responsiveness on smaller graphs.
    #[wasm_bindgen]
    pub fn for_responsive_ui() -> Self {
        Self {
            node_threshold: 1_000,
            depth_threshold: 3,
            progress_interval_ms: 100,
        }
    }
}

impl Default for GraphWorkerConfig {
    fn default() -> Self {
        Self {
            node_threshold: 5_000,
            depth_threshold: 4,
            progress_interval_ms: 100,
        }
    }
}

/// Progress information for long-running traversals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[wasm_bindgen]
#[allow(clippy::unsafe_derive_deserialize)]
pub struct TraversalProgress {
    /// Number of nodes visited so far.
    pub visited_count: usize,
    /// Estimated total nodes to visit (heuristic).
    pub estimated_total: usize,
    /// Current traversal depth.
    pub current_depth: usize,
    /// Whether the traversal is complete.
    pub is_complete: bool,
    /// Whether the traversal was cancelled.
    pub is_cancelled: bool,
}

#[wasm_bindgen]
impl TraversalProgress {
    /// Creates a new progress report.
    #[wasm_bindgen(constructor)]
    pub fn new(visited: usize, estimated: usize, depth: usize) -> Self {
        Self {
            visited_count: visited,
            estimated_total: estimated,
            current_depth: depth,
            is_complete: false,
            is_cancelled: false,
        }
    }

    /// Returns the completion percentage (0-100).
    #[wasm_bindgen(getter)]
    pub fn percentage(&self) -> f64 {
        if self.estimated_total == 0 {
            return 0.0;
        }
        (self.visited_count as f64 / self.estimated_total as f64 * 100.0).min(100.0)
    }

    /// Converts to JSON for postMessage.
    #[wasm_bindgen]
    pub fn to_json(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(self).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

/// Determines whether a traversal should be offloaded to a Web Worker.
///
/// # Arguments
/// * `node_count` - Total nodes in the graph
/// * `max_depth` - Maximum traversal depth requested
/// * `config` - Optional configuration (uses defaults if None)
///
/// # Returns
/// `true` if the operation should use a Web Worker
#[wasm_bindgen]
pub fn should_use_worker(
    node_count: usize,
    max_depth: usize,
    config: Option<GraphWorkerConfig>,
) -> bool {
    let cfg = config.unwrap_or_default();
    node_count >= cfg.node_threshold || max_depth >= cfg.depth_threshold
}

/// Estimates the number of nodes that will be visited during traversal.
///
/// Uses a heuristic based on graph density and max depth.
#[wasm_bindgen]
pub fn estimate_traversal_size(node_count: usize, edge_count: usize, max_depth: usize) -> usize {
    if node_count == 0 {
        return 0;
    }

    // Average degree
    let avg_degree = edge_count as f64 / node_count as f64;

    // Estimate: each level has avg_degree more nodes (with diminishing returns)
    let mut estimated = 1.0;
    let mut level_size = 1.0;

    for _ in 0..max_depth {
        level_size *= avg_degree * 0.7; // 0.7 factor for already-visited nodes
        estimated += level_size;
    }

    (estimated as usize).min(node_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GraphWorkerConfig::default();
        assert_eq!(config.node_threshold, 5_000);
        assert_eq!(config.depth_threshold, 4);
        assert_eq!(config.progress_interval_ms, 100);
    }

    #[test]
    fn test_should_use_worker_by_nodes() {
        let config = GraphWorkerConfig {
            node_threshold: 1000,
            depth_threshold: 10,
            ..Default::default()
        };

        assert!(!should_use_worker(500, 2, Some(config.clone())));
        assert!(should_use_worker(1500, 2, Some(config)));
    }

    #[test]
    fn test_should_use_worker_by_depth() {
        let config = GraphWorkerConfig {
            node_threshold: 10_000,
            depth_threshold: 5,
            ..Default::default()
        };

        assert!(!should_use_worker(100, 3, Some(config.clone())));
        assert!(should_use_worker(100, 6, Some(config)));
    }

    #[test]
    fn test_progress_percentage() {
        let progress = TraversalProgress::new(50, 100, 2);
        assert!((progress.percentage() - 50.0).abs() < 0.01);

        let progress_zero = TraversalProgress::new(0, 0, 0);
        assert!((progress_zero.percentage() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_traversal_size() {
        // Empty graph
        assert_eq!(estimate_traversal_size(0, 0, 5), 0);

        // Single node
        assert_eq!(estimate_traversal_size(1, 0, 5), 1);

        // Small graph with depth 1
        let estimate = estimate_traversal_size(100, 200, 1);
        assert!(estimate > 0 && estimate <= 100);

        // Larger depth
        let estimate_deep = estimate_traversal_size(1000, 3000, 5);
        assert!(estimate_deep <= 1000);
    }
}
