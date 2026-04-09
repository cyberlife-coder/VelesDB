//! Persistence methods for `NativeHnswIndex` (save/load).
//!
//! Extracted from `native_index.rs` to keep file NLOC under 500.
//! All public API signatures remain unchanged.

use super::native_index::NativeHnswIndex;
use super::native_inner::NativeHnswInner;
use super::params::HnswParams;
use super::persistence::{self, HnswMappingsData, HnswMeta};
use super::sharded_mappings::ShardedMappings;
use crate::distance::DistanceMetric;
use parking_lot::RwLock;
use std::path::Path;

impl NativeHnswIndex {
    /// Saves the index to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;

        // Save HNSW graph
        let inner = self.inner.read();
        inner.file_dump(path, "native_hnsw")?;

        // Save mappings
        let (id_to_idx, idx_to_id, next_idx) = self.mappings.as_parts();
        persistence::save_mappings(
            path,
            &HnswMappingsData {
                id_to_idx,
                idx_to_id,
                next_idx,
            },
        )?;

        // Save or clean up vectors (shared helper)
        persistence::save_or_cleanup_vectors(path, self.enable_vector_storage, &self.vectors)?;

        // Save metadata
        persistence::save_meta(
            path,
            &HnswMeta {
                dimension: self.dimension,
                metric: self.metric,
                enable_vector_storage: self.enable_vector_storage,
                storage_mode: self.inner.read().storage_mode(),
            },
        )?;

        Ok(())
    }

    /// Loads the index from disk.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the index directory
    /// * `_dimension` - Ignored (read from metadata) - for API compatibility
    /// * `_metric` - Ignored (read from metadata) - for API compatibility
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail or data is corrupted.
    pub fn load<P: AsRef<Path>>(
        path: P,
        _dimension: usize,
        _metric: DistanceMetric,
    ) -> std::io::Result<Self> {
        let path = path.as_ref();

        let meta = persistence::load_meta(path)?;

        // Load HNSW graph (with storage mode for RaBitQ backend support)
        let inner = NativeHnswInner::file_load_with_storage_mode(
            path,
            "native_hnsw",
            meta.metric,
            meta.dimension,
            meta.storage_mode,
        )?;

        // Load mappings
        let mappings_data = persistence::load_mappings(path)?;
        let mappings = ShardedMappings::from_parts(
            mappings_data.id_to_idx,
            mappings_data.idx_to_id,
            mappings_data.next_idx,
        );

        // Load vectors (gracefully disables if file missing)
        let (vectors, enable_vector_storage) = persistence::load_vectors_or_disable(path, &meta)?;

        Ok(Self {
            dimension: meta.dimension,
            metric: meta.metric,
            inner: RwLock::new(inner),
            mappings,
            vectors,
            enable_vector_storage,
            params: HnswParams::auto(meta.dimension),
        })
    }
}
