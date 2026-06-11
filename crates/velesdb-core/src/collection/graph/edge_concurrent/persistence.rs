//! `ConcurrentEdgeStore` persistence (serialization/deserialization).
//!
//! Extracted from `edge_concurrent/mod.rs` to reduce NLOC below the 500 threshold.

use super::super::edge::EdgeStore;
use super::ConcurrentEdgeStore;

impl ConcurrentEdgeStore {
    /// Builds a `ConcurrentEdgeStore` from a persisted `EdgeStore`.
    ///
    /// Re-distributes edges across shards based on source node ID.
    ///
    /// Issue #905: uses [`add_edges_batch`](Self::add_edges_batch) (one
    /// `edge_ids` write-lock acquisition for the whole set, snapshot
    /// invalidated once) instead of a per-edge `add_edge` loop (one lock
    /// cycle + one snapshot-dirty flip per edge). The single
    /// [`build_read_snapshot`](Self::build_read_snapshot) at the end performs
    /// exactly one O(N+E) CSR build for the whole reconstruction.
    #[must_use]
    pub fn from_edge_store(store: &EdgeStore) -> Self {
        let edges: Vec<_> = store.all_edges().into_iter().cloned().collect();
        let concurrent = Self::with_estimated_edges(edges.len());

        let expected = edges.len();
        let added = concurrent.add_edges_batch(edges);
        if added != expected {
            tracing::warn!(
                "skipped {} duplicate edge(s) during CES reconstruction",
                expected - added
            );
        }

        concurrent.build_read_snapshot();
        concurrent
    }

    /// Saves the concurrent edge store to a file.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or file I/O fails.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        self.to_merged_edge_store().save_to_file(path)
    }

    /// Loads a concurrent edge store from a persisted file.
    ///
    /// # Errors
    ///
    /// Returns an error if file I/O or deserialization fails.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let store = EdgeStore::load_from_file(path)?;
        Ok(Self::from_edge_store(&store))
    }

    /// Merges all shards into a single `EdgeStore` for serialization.
    fn to_merged_edge_store(&self) -> EdgeStore {
        let ids = self.edge_ids.read();
        let mut merged = EdgeStore::with_capacity(ids.len(), ids.len());

        for (&edge_id, &source_id) in ids.iter() {
            let shard_idx = self.shard_index(source_id);
            let guard = self.shards[shard_idx].read();
            if let Some(edge) = guard.get_edge(edge_id) {
                let _ = merged.add_edge(edge.clone());
            }
        }
        merged
    }
}
