//! Persistence methods for `NativeHnswIndex` (save/load).
//!
//! Extracted from `native_index.rs` to keep file NLOC under 500.
//! All public API signatures remain unchanged.

use super::native_index::NativeHnswIndex;
use super::native_inner::NativeHnswInner;
use super::params::HnswParams;
use super::persistence::{self, HnswMeta};
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

        // Dump the HNSW graph itself (caller-specific — see persistence::save_sidecars).
        let storage_mode = {
            let inner = self.inner.read();
            inner.file_dump(path, "native_hnsw")?;
            inner.storage_mode()
        };

        // Mappings + vectors + meta in one shared call (RF-DEDUP #448 Group C).
        persistence::save_sidecars(
            path,
            &self.mappings,
            &self.vectors,
            &HnswMeta {
                dimension: self.dimension,
                metric: self.metric,
                enable_vector_storage: self.enable_vector_storage,
                storage_mode,
                // `save_sidecars` overwrites this with the bumped generation (#617).
                generation: 0,
            },
        )
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

        // Load HNSW graph (with storage mode for RaBitQ backend support).
        let inner = NativeHnswInner::file_load_with_storage_mode(
            path,
            "native_hnsw",
            meta.metric,
            meta.dimension,
            meta.storage_mode,
        )?;

        // Mappings + vectors in one shared call (RF-DEDUP #448 Group C).
        let (mappings, vectors, enable_vector_storage) = persistence::load_sidecars(path, &meta)?;

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
