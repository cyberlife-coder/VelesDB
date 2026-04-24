//! Edge removal operations for `EdgeStore`.
//!
//! Extracted from `edge.rs` to keep file NLOC under the 500 threshold.
//! Contains `remove_edge`, `remove_node_edges`, and the internal
//! `purge_*` index-cleanup helpers.

use super::edge::EdgeStore;

impl EdgeStore {
    /// Removes an edge by ID.
    ///
    /// Cleans up all indices: outgoing, incoming, by_label, and outgoing_by_label.
    pub fn remove_edge(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            let source = edge.source();
            self.purge_outgoing_index(edge_id, source);
            self.purge_incoming_index(edge_id, edge.target());
            self.purge_label_indices(edge_id, source, edge.label());
            // Invalidate CSR snapshot (G1).
            self.csr_snapshot = None;
        }
    }

    /// Removes an edge by ID, only cleaning the outgoing index.
    ///
    /// Used by `ConcurrentEdgeStore` for cross-shard cleanup.
    /// Also cleans up label indices since they are maintained by source shard.
    pub fn remove_edge_outgoing_only(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            let source = edge.source();
            self.purge_outgoing_index(edge_id, source);
            self.purge_label_indices(edge_id, source, edge.label());
            // Invalidate CSR snapshot (G1).
            self.csr_snapshot = None;
        }
    }

    /// Removes an edge by ID, only cleaning the incoming index.
    ///
    /// Used by `ConcurrentEdgeStore` for cross-shard cleanup.
    pub fn remove_edge_incoming_only(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            self.purge_incoming_index(edge_id, edge.target());
            // Invalidate CSR snapshot (G1).
            self.csr_snapshot = None;
        }
    }

    /// Removes all edges connected to a node (cascade delete).
    ///
    /// Removes both outgoing and incoming edges, cleaning up all indices
    /// including label indices (EPIC-019 US-003).
    pub fn remove_node_edges(&mut self, node_id: u64) {
        // Collect edge IDs to remove (outgoing)
        let outgoing_ids: Vec<u64> = self.outgoing.remove(&node_id).unwrap_or_default();

        // Collect edge IDs to remove (incoming)
        let incoming_ids: Vec<u64> = self.incoming.remove(&node_id).unwrap_or_default();

        // Remove outgoing edges: clean incoming + label indices for each
        for edge_id in outgoing_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                self.purge_incoming_index(edge_id, edge.target());
                self.purge_label_indices(edge_id, node_id, edge.label());
            }
        }

        // Remove incoming edges: clean outgoing + label indices for each
        for edge_id in incoming_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                let source = edge.source();
                self.purge_outgoing_index(edge_id, source);
                self.purge_label_indices(edge_id, source, edge.label());
            }
        }

        // Invalidate CSR snapshot (G1).
        self.csr_snapshot = None;
    }

    /// Removes `edge_id` from the incoming index of `target_node`.
    #[inline]
    fn purge_incoming_index(&mut self, edge_id: u64, target_node: u64) {
        if let Some(ids) = self.incoming.get_mut(&target_node) {
            ids.retain(|&id| id != edge_id);
        }
    }

    /// Removes `edge_id` from the outgoing index of `source_node`.
    #[inline]
    fn purge_outgoing_index(&mut self, edge_id: u64, source_node: u64) {
        if let Some(ids) = self.outgoing.get_mut(&source_node) {
            ids.retain(|&id| id != edge_id);
        }
    }

    /// Removes `edge_id` from the `by_label` and `outgoing_by_label` indices (US-003).
    #[inline]
    fn purge_label_indices(&mut self, edge_id: u64, source_node: u64, label: &str) {
        if let Some(ids) = self.by_label.get_mut(label) {
            ids.retain(|&id| id != edge_id);
        }
        if let Some(ids) = self
            .outgoing_by_label
            .get_mut(&(source_node, label.to_string()))
        {
            ids.retain(|&id| id != edge_id);
        }
    }
}
