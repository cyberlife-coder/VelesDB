//! Edge removal operations for `EdgeStore`.
//!
//! Extracted from `edge.rs` to keep file NLOC under the 500 threshold.
//! Contains `remove_edge`, `remove_node_edges`, and the internal
//! `purge_*` index-cleanup helpers.
//!
//! Every `purge_*` helper drops the map key itself once its bucket empties,
//! not just the `edge_id` inside it — a node or label that no longer has any
//! edge must not keep a permanently empty `Vec` alive. Node ids are
//! effectively never reused, so leaving the key behind means `outgoing` and
//! `incoming` (persisted fields, unlike the `serde(skip)` label indices)
//! grow without bound under add/remove churn and never shrink back down.

use super::edge::EdgeStore;
use super::label_table::LabelId;

impl EdgeStore {
    /// Removes an edge by ID.
    ///
    /// Cleans up all indices: outgoing, incoming, by_label, and outgoing_by_label.
    pub fn remove_edge(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            let source = edge.source();
            let label_id = self.label_table.get_id(edge.label());
            self.purge_outgoing_index(edge_id, source);
            self.purge_incoming_index(edge_id, edge.target(), label_id);
            self.purge_label_indices(edge_id, source, label_id);
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
            let label_id = self.label_table.get_id(edge.label());
            self.purge_outgoing_index(edge_id, source);
            self.purge_label_indices(edge_id, source, label_id);
            // Invalidate CSR snapshot (G1).
            self.csr_snapshot = None;
        }
    }

    /// Removes an edge by ID, only cleaning the incoming index.
    ///
    /// Used by `ConcurrentEdgeStore` for cross-shard cleanup.
    pub fn remove_edge_incoming_only(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            let label_id = self.label_table.get_id(edge.label());
            self.purge_incoming_index(edge_id, edge.target(), label_id);
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
                let label_id = self.label_table.get_id(edge.label());
                self.purge_incoming_index(edge_id, edge.target(), label_id);
                self.purge_label_indices(edge_id, node_id, label_id);
            }
        }

        // Remove incoming edges: clean outgoing + label indices for each.
        // `incoming` was drained wholesale above; the label mirror still
        // needs its per-(node, label) entries purged.
        for edge_id in incoming_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                let source = edge.source();
                let label_id = self.label_table.get_id(edge.label());
                self.purge_outgoing_index(edge_id, source);
                self.purge_label_indices(edge_id, source, label_id);
                self.purge_incoming_index(edge_id, node_id, label_id);
            }
        }

        // Invalidate CSR snapshot (G1).
        self.csr_snapshot = None;
    }

    /// Removes `edge_id` from the incoming index of `target_node`.
    ///
    /// `label_id` is `None` only when the edge's label was never interned —
    /// in that case the label maps hold no entry for it either (table and
    /// maps are populated together), so skipping the label purge is exact,
    /// not merely defensive.
    #[inline]
    fn purge_incoming_index(&mut self, edge_id: u64, target_node: u64, label_id: Option<LabelId>) {
        if let Some(ids) = self.incoming.get_mut(&target_node) {
            ids.retain(|&id| id != edge_id);
            if ids.is_empty() {
                self.incoming.remove(&target_node);
            }
        }
        let Some(label_id) = label_id else { return };
        if let Some(ids) = self.incoming_by_label.get_mut(&(target_node, label_id)) {
            ids.retain(|&id| id != edge_id);
            if ids.is_empty() {
                self.incoming_by_label.remove(&(target_node, label_id));
            }
        }
    }

    /// Removes `edge_id` from the outgoing index of `source_node`.
    #[inline]
    fn purge_outgoing_index(&mut self, edge_id: u64, source_node: u64) {
        if let Some(ids) = self.outgoing.get_mut(&source_node) {
            ids.retain(|&id| id != edge_id);
            if ids.is_empty() {
                self.outgoing.remove(&source_node);
            }
        }
    }

    /// Removes `edge_id` from the `by_label` and `outgoing_by_label` indices (US-003).
    ///
    /// Same `None` contract as [`Self::purge_incoming_index`].
    #[inline]
    fn purge_label_indices(&mut self, edge_id: u64, source_node: u64, label_id: Option<LabelId>) {
        let Some(label_id) = label_id else { return };
        if let Some(ids) = self.by_label.get_mut(&label_id) {
            ids.retain(|&id| id != edge_id);
            if ids.is_empty() {
                self.by_label.remove(&label_id);
            }
        }
        if let Some(ids) = self.outgoing_by_label.get_mut(&(source_node, label_id)) {
            ids.retain(|&id| id != edge_id);
            if ids.is_empty() {
                self.outgoing_by_label.remove(&(source_node, label_id));
            }
        }
    }
}
