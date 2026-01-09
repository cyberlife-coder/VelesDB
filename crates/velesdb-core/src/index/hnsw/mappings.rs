//! ID mappings for HNSW index (DEPRECATED).
//!
//! This module provides bidirectional mapping between external IDs (u64)
//! and internal HNSW indices (usize).
//!
//! **Note**: This module is deprecated in favor of `ShardedMappings` which
//! provides lock-free concurrent access. Kept for backwards compatibility
//! with existing tests and potential future use.

#![allow(dead_code)]

use std::collections::HashMap;

/// ID mappings for HNSW index.
///
/// Groups all mapping-related data under a single lock to reduce
/// lock contention during parallel insertions.
#[derive(Debug, Clone, Default)]
pub struct HnswMappings {
    /// Mapping from external IDs to internal indices.
    id_to_idx: HashMap<u64, usize>,
    /// Mapping from internal indices to external IDs.
    idx_to_id: HashMap<usize, u64>,
    /// Next available internal index.
    next_idx: usize,
}

impl HnswMappings {
    /// Creates new empty mappings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates mappings from existing data (for deserialization).
    #[must_use]
    pub fn from_parts(
        id_to_idx: HashMap<u64, usize>,
        idx_to_id: HashMap<usize, u64>,
        next_idx: usize,
    ) -> Self {
        Self {
            id_to_idx,
            idx_to_id,
            next_idx,
        }
    }

    /// Registers an ID and returns its internal index.
    /// Returns `None` if the ID already exists.
    pub fn register(&mut self, id: u64) -> Option<usize> {
        if self.id_to_idx.contains_key(&id) {
            return None;
        }
        let idx = self.next_idx;
        self.next_idx += 1;
        self.id_to_idx.insert(id, idx);
        self.idx_to_id.insert(idx, id);
        Some(idx)
    }

    /// Removes an ID and returns its internal index if it existed.
    pub fn remove(&mut self, id: u64) -> Option<usize> {
        if let Some(idx) = self.id_to_idx.remove(&id) {
            self.idx_to_id.remove(&idx);
            Some(idx)
        } else {
            None
        }
    }

    /// Gets the internal index for an external ID.
    #[must_use]
    pub fn get_idx(&self, id: u64) -> Option<usize> {
        self.id_to_idx.get(&id).copied()
    }

    /// Gets the external ID for an internal index.
    #[must_use]
    pub fn get_id(&self, idx: usize) -> Option<u64> {
        self.idx_to_id.get(&idx).copied()
    }

    /// Returns the number of registered IDs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.id_to_idx.len()
    }

    /// Returns true if no IDs are registered.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.id_to_idx.is_empty()
    }

    /// Returns references for serialization.
    #[must_use]
    pub fn as_parts(&self) -> (&HashMap<u64, usize>, &HashMap<usize, u64>, usize) {
        (&self.id_to_idx, &self.idx_to_id, self.next_idx)
    }
}
