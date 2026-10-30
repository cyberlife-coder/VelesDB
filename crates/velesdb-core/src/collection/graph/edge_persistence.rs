//! `EdgeStore` persistence (serialization/deserialization) methods.
//!
//! Extracted from `edge.rs` to reduce NLOC below the 500 threshold. The
//! snapshot form itself is `edge::snapshot_wire`.

use super::csr_snapshot::{CsrSnapshot, SnapshotBuilder};
use super::edge::snapshot_wire;
use super::edge::EdgeStore;

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
        self.csr_snapshot = Some(SnapshotBuilder::build(self));
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

// Persistence through the private snapshot form (`edge::snapshot_wire`):
// `EdgeStore` has no serde representation of its own.
impl EdgeStore {
    /// Serializes the edge store to its `edge_store.bin` snapshot bytes.
    ///
    /// # Errors
    /// Returns an error if serialization fails, including when an edge's
    /// properties nest deeper than
    /// [`MAX_JSON_NESTING`](crate::validation::MAX_JSON_NESTING), which the
    /// snapshot could not read back.
    pub fn to_bytes(&self) -> std::result::Result<Vec<u8>, postcard::Error> {
        snapshot_wire::to_bytes(self)
    }

    /// Deserializes an edge store from snapshot bytes.
    ///
    /// Rebuilds the (unserialized) label table and label indices, so by-label
    /// queries work on the returned store exactly as on the one serialized —
    /// they would otherwise come back empty (#2089).
    ///
    /// # Errors
    /// Returns an error if deserialization fails (e.g., corrupted data).
    pub fn from_bytes(bytes: &[u8]) -> std::result::Result<Self, postcard::Error> {
        let mut store = snapshot_wire::from_bytes(bytes)?;
        store
            .rebuild_label_indexes()
            // Unreachable in practice (u32::MAX distinct labels); surfaced as
            // a deserialization failure rather than a silently unindexed store.
            .map_err(|_| postcard::Error::SerdeDeCustom)?;
        Ok(store)
    }

    /// Saves the edge store to a file **atomically** (see
    /// `storage::atomic_write`): a crash mid-write leaves the previous
    /// snapshot intact.
    ///
    /// # Errors
    /// Returns an error if serialization or file I/O fails.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = self.to_bytes().map_err(invalid_data)?;
        crate::storage::atomic_write::atomic_write(path, &bytes)
    }

    /// Loads an edge store from a file.
    ///
    /// Rebuilds the (unserialized) label table and label indices, then
    /// builds a CSR snapshot for zero-copy BFS traversal (G1).
    ///
    /// # Errors
    /// Returns the I/O error of the read, or an
    /// [`InvalidData`](std::io::ErrorKind::InvalidData) error for every
    /// content the snapshot cannot be decoded from.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let mut store = Self::from_bytes(&bytes).map_err(invalid_data)?;
        store.build_read_snapshot();
        Ok(store)
    }
}

fn invalid_data(e: postcard::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}
