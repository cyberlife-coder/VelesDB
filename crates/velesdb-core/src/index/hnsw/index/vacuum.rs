//! Vacuum and maintenance operations for HnswIndex.

use super::{HnswIndex, HnswInner};
use crate::index::hnsw::params::HnswParams;
use crate::index::hnsw::sharded_mappings::ShardedMappings;
use crate::index::hnsw::sharded_vectors::ShardedVectors;
use std::mem::ManuallyDrop;

/// Errors that can occur during vacuum operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VacuumError {
    /// Vector storage is disabled, cannot rebuild index
    VectorStorageDisabled,
}

impl std::fmt::Display for VacuumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VectorStorageDisabled => {
                write!(f, "Cannot vacuum: vector storage is disabled (use new() instead of new_fast_insert())")
            }
        }
    }
}

impl std::error::Error for VacuumError {}

impl HnswIndex {
    /// Returns the number of tombstones (soft-deleted entries) in the index.
    ///
    /// Tombstones are entries that have been removed from mappings but still
    /// exist in the underlying HNSW graph. High tombstone count degrades
    /// search performance.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// // Insert and delete some vectors...
    /// if index.tombstone_ratio() > 0.2 {
    ///     index.needs_vacuum(); // Consider rebuilding
    /// }
    /// ```
    #[must_use]
    pub fn tombstone_count(&self) -> usize {
        // Total inserted = next_idx in mappings (monotonic counter)
        // Active = mappings.len()
        // Tombstones = Total - Active
        let total_inserted = self.mappings.next_idx();
        let active = self.mappings.len();
        total_inserted.saturating_sub(active)
    }

    /// Returns the tombstone ratio (0.0 = clean, 1.0 = 100% deleted).
    ///
    /// Use this to decide when to trigger a vacuum/rebuild operation.
    /// A ratio > 0.2 (20%) is a reasonable threshold for considering vacuum.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Acceptable precision loss for ratio calculation
    pub fn tombstone_ratio(&self) -> f64 {
        let total = self.mappings.next_idx();
        if total == 0 {
            return 0.0;
        }
        let tombstones = self.tombstone_count();
        tombstones as f64 / total as f64
    }

    /// Returns true if the index has significant fragmentation and would
    /// benefit from a vacuum/rebuild operation.
    ///
    /// Current threshold: 20% tombstones
    #[must_use]
    pub fn needs_vacuum(&self) -> bool {
        self.tombstone_ratio() > 0.2
    }

    /// Rebuilds the HNSW index, removing all tombstones.
    ///
    /// This creates a new HNSW graph containing only the active vectors,
    /// eliminating fragmentation and improving search performance.
    ///
    /// # Important
    ///
    /// - This operation is **blocking** and may take significant time for large indices
    /// - The index remains readable during rebuild (copy-on-write pattern)
    /// - Requires `enable_vector_storage = true` (vectors must be stored)
    ///
    /// # Returns
    ///
    /// - `Ok(count)` - Number of vectors in the rebuilt index
    /// - `Err` - If vector storage is disabled or rebuild fails
    ///
    /// # Errors
    ///
    /// Returns `VacuumError::VectorStorageDisabled` if the index was created
    /// with `new_fast_insert()` mode, which disables vector storage.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// // ... insert and delete many vectors ...
    ///
    /// if index.needs_vacuum() {
    ///     let count = index.vacuum()?;
    ///     println!("Rebuilt index with {} vectors", count);
    /// }
    /// ```
    pub fn vacuum(&self) -> Result<usize, VacuumError> {
        if !self.enable_vector_storage {
            return Err(VacuumError::VectorStorageDisabled);
        }

        // 1. Collect all active vectors (copy-on-write snapshot)
        let active_vectors: Vec<(u64, Vec<f32>)> = self
            .mappings
            .iter()
            .filter_map(|(id, idx)| self.vectors.get(idx).map(|vec| (id, vec)))
            .collect();

        let count = active_vectors.len();

        if count == 0 {
            return Ok(0);
        }

        // 2. Create new HNSW graph with auto-tuned parameters
        let params = HnswParams::auto(self.dimension);
        let new_inner = HnswInner::new(
            self.metric,
            params.max_connections,
            count.max(1000), // max_elements with reasonable minimum
            params.ef_construction,
            self.dimension,
        );

        // 3. Create new mappings and vectors
        let new_mappings = ShardedMappings::with_capacity(count);
        let new_vectors = ShardedVectors::new(self.dimension);

        // 4. Bulk insert into new structures
        let refs_for_hnsw: Vec<(&Vec<f32>, usize)> = active_vectors
            .iter()
            .enumerate()
            .map(|(idx, (id, vec))| {
                // Register in new mappings
                new_mappings.register(*id);
                // Store in new vectors
                new_vectors.insert(idx, vec);
                (vec, idx)
            })
            .collect();

        // 5. Parallel insert into new HNSW
        new_inner.parallel_insert(&refs_for_hnsw);

        // 6. Atomic swap (replace old with new)
        {
            let mut inner_guard = self.inner.write();
            // SAFETY: ManuallyDrop::drop is safe when exclusive ownership is guaranteed.
            // - Condition 1: We hold exclusive write lock on inner_guard (no other access possible)
            // - Condition 2: This is called exactly once before replacement (no double-drop)
            // - Condition 3: The old value is immediately replaced with new_inner (no use-after-free)
            // Reason: Explicit drop required before assignment to ManuallyDrop field.
            unsafe {
                ManuallyDrop::drop(&mut *inner_guard);
            }
            // Replace with new
            *inner_guard = ManuallyDrop::new(new_inner);
        }

        // 7. Swap mappings and vectors
        // Note: ShardedMappings/ShardedVectors use interior mutability,
        // so we need to clear and repopulate
        self.mappings.clear();
        self.vectors.clear();

        for (id, vec) in active_vectors {
            if let Some(idx) = self.mappings.register(id) {
                self.vectors.insert(idx, &vec);
            }
        }

        Ok(count)
    }
}
