//! Bidirectional BFS traversal for graph queries.
//!
//! Combines forward and reverse BFS to discover nodes reachable in both
//! directions from a source node. Uses target-only deduplication to avoid
//! reporting the same node twice.
//!
//! Extracted from [`super::traversal`] to isolate the bidirectional
//! composition from the core BFS machinery.

use rustc_hash::FxHashSet;

use super::edge::EdgeStore;
use super::traversal::{bfs_traverse, bfs_traverse_reverse, TraversalConfig, TraversalResult};

/// Performs bidirectional BFS (follows both directions).
///
/// # Deduplication strategy
///
/// Uses target-only dedup via `FxHashSet<u64>`: each target node appears
/// at most once across forward and reverse results. This was chosen over
/// the prior path+target dedup because:
/// - O(1) `HashSet::contains` vs O(n) linear scan per reverse result.
/// - Cleaner output — a node reached by both forward and reverse edges
///   appears once (via whichever direction discovered it first).
/// - The forward pass populates the `seen` set; reverse results are skipped
///   if their `target_id` is already present.
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
    // Build O(1) dedup set from forward results to avoid O(n) linear scan per reverse result.
    let seen: FxHashSet<u64> = forward.iter().map(|r| r.target_id).collect();
    results.extend(forward);

    // Reverse traversal — skip targets already reached by forward BFS
    if results.len() < config.limit {
        let reverse = bfs_traverse_reverse(edge_store, source_id, &config_half);
        for r in reverse {
            if results.len() >= config.limit {
                break;
            }
            if !seen.contains(&r.target_id) {
                results.push(r);
            }
        }
    }

    results.truncate(config.limit);
    results
}
