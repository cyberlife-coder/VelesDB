//! `EdgeStore` persistence (serialization/deserialization) methods.
//!
//! Extracted from `edge.rs` to reduce NLOC below the 500 threshold.

use super::csr_snapshot::{CsrSnapshot, SnapshotBuilder};
use super::edge::EdgeStore;
use super::helpers::PostcardPersistence;
use super::label_table::LabelTable;

// ---------------------------------------------------------------------------
// CSR snapshot methods (G1: zero-copy BFS)
// ---------------------------------------------------------------------------
impl EdgeStore {
    /// Builds a CSR snapshot from the current outgoing index.
    ///
    /// This pre-computes contiguous arrays of target IDs, edge IDs, and
    /// interned labels for all source nodes. After calling this, BFS
    /// traversal uses `with_neighbors()` for zero-copy `&[u64]` access
    /// instead of cloning full `GraphEdge` objects.
    ///
    /// # When to call
    ///
    /// - After loading from disk (graph is ready for reads)
    /// - After a batch of mutations, before a read-heavy phase
    ///
    /// The snapshot is automatically invalidated by any write operation.
    pub fn build_read_snapshot(&mut self) {
        let label_table = LabelTable::new();
        self.csr_snapshot = Some(SnapshotBuilder::build(self, &label_table));
    }

    /// Returns a reference to the CSR snapshot, if built.
    #[must_use]
    #[inline]
    pub fn csr_snapshot(&self) -> Option<&CsrSnapshot> {
        self.csr_snapshot.as_ref()
    }

    /// Returns `true` if a CSR snapshot is available for zero-copy reads.
    #[must_use]
    #[inline]
    pub fn has_csr_snapshot(&self) -> bool {
        self.csr_snapshot.is_some()
    }

    /// Provides zero-copy access to neighbor target IDs via a callback.
    #[inline]
    pub fn with_neighbors<F, R>(&self, source_id: u64, f: F) -> R
    where
        F: FnOnce(&[u64]) -> R,
    {
        if let Some(snapshot) = &self.csr_snapshot {
            f(snapshot.neighbors(source_id))
        } else {
            let ids: Vec<u64> = self
                .get_outgoing(source_id)
                .iter()
                .map(|e| e.target())
                .collect();
            f(&ids)
        }
    }

    /// Provides zero-copy access to `(target_id, edge_id)` pairs via callback.
    #[inline]
    pub fn with_neighbor_edges<F, R>(&self, source_id: u64, f: F) -> R
    where
        F: FnOnce(&[u64], &[u64]) -> R,
    {
        if let Some(snapshot) = &self.csr_snapshot {
            f(snapshot.neighbors(source_id), snapshot.edge_ids(source_id))
        } else {
            let edges = self.get_outgoing(source_id);
            let targets: Vec<u64> = edges.iter().map(|e| e.target()).collect();
            let eids: Vec<u64> = edges.iter().map(|e| e.id()).collect();
            f(&targets, &eids)
        }
    }
}

impl PostcardPersistence for EdgeStore {}

// Inherent persistence methods that delegate to `PostcardPersistence`.
impl EdgeStore {
    /// Serializes the edge store to bytes using `postcard`.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub fn to_bytes(&self) -> std::result::Result<Vec<u8>, postcard::Error> {
        <Self as PostcardPersistence>::to_bytes(self)
    }

    /// Deserializes an edge store from bytes.
    ///
    /// # Errors
    /// Returns an error if deserialization fails (e.g., corrupted data).
    pub fn from_bytes(bytes: &[u8]) -> std::result::Result<Self, postcard::Error> {
        <Self as PostcardPersistence>::from_bytes(bytes)
    }

    /// Saves the edge store to a file.
    ///
    /// # Errors
    /// Returns an error if serialization or file I/O fails.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        <Self as PostcardPersistence>::save_to_file(self, path)
    }

    /// Loads an edge store from a file.
    ///
    /// Automatically builds a CSR snapshot after loading for zero-copy
    /// BFS traversal (G1).
    ///
    /// # Errors
    /// Returns an error if file I/O or deserialization fails.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let mut store = <Self as PostcardPersistence>::load_from_file(path)?;
        store.build_read_snapshot();
        Ok(store)
    }
}
