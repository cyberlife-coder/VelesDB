//! Node-cascade cleanup helpers for `ConcurrentEdgeStore`.
//!
//! Extracted from the main module for single-responsibility: these private
//! helpers implement the multi-shard cleanup performed by `remove_node_edges`
//! — collecting a node's outgoing/incoming edges, gathering the affected
//! shards in ascending lock order, removing the dangling cross-shard
//! half-edges, and deregistering the global edge-id entries.
//!
//! Lock ordering is owned by `remove_node_edges` (in the main module), which
//! acquires `edge_ids` first and then the shard write guards in ascending
//! order before calling these helpers; the helpers themselves only operate on
//! guards already held by the caller.

use super::{ConcurrentEdgeStore, EdgeStore};
use parking_lot::RwLockWriteGuard;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

impl ConcurrentEdgeStore {
    /// Collects all outgoing and incoming edges for a node (read-only).
    #[allow(clippy::type_complexity)] // Reason: tuple of (outgoing, incoming) edge lists is clear in context
    pub(super) fn collect_node_edges(
        &self,
        node_shard: usize,
        node_id: u64,
    ) -> (Vec<(u64, u64)>, Vec<(u64, u64)>) {
        let guard = self.shards[node_shard].read();
        let outgoing: Vec<_> = guard
            .get_outgoing(node_id)
            .iter()
            .map(|e| (e.id(), e.target()))
            .collect();
        let incoming: Vec<_> = guard
            .get_incoming(node_id)
            .iter()
            .map(|e| (e.id(), e.source()))
            .collect();
        (outgoing, incoming)
    }

    /// Gathers the set of shard indices that need cleanup (sorted ascending for lock ordering).
    pub(super) fn gather_affected_shards(
        &self,
        node_shard: usize,
        outgoing: &[(u64, u64)],
        incoming: &[(u64, u64)],
    ) -> BTreeSet<usize> {
        let mut shards = BTreeSet::new();
        shards.insert(node_shard);
        for (_, target) in outgoing {
            shards.insert(self.shard_index(*target));
        }
        for (_, source) in incoming {
            shards.insert(self.shard_index(*source));
        }
        shards
    }

    /// Cleans up edges in all affected shards.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn cleanup_shard_edges(
        &self,
        guards: &mut [(usize, RwLockWriteGuard<'_, EdgeStore>)],
        node_shard: usize,
        node_id: u64,
        outgoing: &[(u64, u64)],
        incoming: &[(u64, u64)],
    ) {
        for (shard_idx, guard) in guards {
            if *shard_idx == node_shard {
                guard.remove_node_edges(node_id);
            } else {
                self.cleanup_cross_shard_edges(*shard_idx, guard, outgoing, incoming);
            }
        }
    }

    /// Removes the dangling half-edges that terminate in `shard_idx` for a cross-shard cleanup.
    fn cleanup_cross_shard_edges(
        &self,
        shard_idx: usize,
        guard: &mut parking_lot::RwLockWriteGuard<'_, super::EdgeStore>,
        outgoing: &[(u64, u64)],
        incoming: &[(u64, u64)],
    ) {
        for (edge_id, target) in outgoing {
            if self.shard_index(*target) == shard_idx {
                guard.remove_edge_incoming_only(*edge_id);
            }
        }
        for (edge_id, source) in incoming {
            if self.shard_index(*source) == shard_idx {
                guard.remove_edge_outgoing_only(*edge_id);
            }
        }
    }

    /// Removes edge IDs from the global registry, deduplicating.
    #[allow(clippy::unused_self)] // Reason: method on ConcurrentEdgeStore for API consistency
    pub(super) fn deregister_edge_ids(
        &self,
        ids: &mut FxHashMap<u64, u64>,
        outgoing: &[(u64, u64)],
        incoming: &[(u64, u64)],
    ) {
        let mut removed: FxHashSet<u64> = FxHashSet::default();
        for (edge_id, _) in outgoing {
            if removed.insert(*edge_id) {
                ids.remove(edge_id);
            }
        }
        for (edge_id, _) in incoming {
            if removed.insert(*edge_id) {
                ids.remove(edge_id);
            }
        }
    }
}
