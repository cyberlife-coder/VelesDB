//! RF-2.3: HNSW index persistence (save/load).
//!
//! This module handles serialization and deserialization of HNSW indices
//! to and from disk, including the graph structure and ID mappings.

use super::inner::HnswInner;
use super::sharded_mappings::ShardedMappings;
use crate::distance::DistanceMetric;
use hnsw_rs::hnswio::HnswIo;
use hnsw_rs::prelude::*;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::path::Path;

/// Saves the HNSW index and ID mappings to the specified directory.
///
/// # File Layout
///
/// ```text
/// <path>/
/// ├── hnsw_index.hnsw.data   # HNSW graph data
/// ├── hnsw_index.hnsw.graph  # HNSW graph structure
/// └── id_mappings.bin        # External ID <-> internal index mappings
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - Directory creation fails
/// - HNSW graph serialization fails
/// - ID mappings serialization fails
// RF-2.3: Will be used by HnswIndex::save after refactoring
#[allow(dead_code)]
pub(super) fn save_index(
    path: &Path,
    inner: &RwLock<ManuallyDrop<HnswInner>>,
    mappings: &ShardedMappings,
) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;

    let basename = "hnsw_index";

    // 1. Save HNSW graph
    let inner_guard = inner.read();
    inner_guard.file_dump(path, basename)?;

    // 2. Save Mappings
    let mappings_path = path.join("id_mappings.bin");
    let file = std::fs::File::create(mappings_path)?;
    let writer = std::io::BufWriter::new(file);

    let (id_to_idx, idx_to_id, next_idx) = mappings.as_parts();

    bincode::serialize_into(writer, &(id_to_idx, idx_to_id, next_idx))
        .map_err(std::io::Error::other)?;

    Ok(())
}

/// Result of loading an HNSW index from disk.
///
/// Contains all components needed to reconstruct an `HnswIndex`.
// RF-2.3: Will be used by HnswIndex::load after refactoring
#[allow(dead_code)]
pub(super) struct LoadedIndex {
    /// The loaded HNSW graph wrapper
    pub inner: HnswInner,
    /// The loaded ID mappings
    pub mappings: ShardedMappings,
    /// The `HnswIo` holder (must outlive `inner`)
    pub io_holder: Box<HnswIo>,
}

/// Loads the HNSW index and ID mappings from the specified directory.
///
/// # Safety
///
/// This function uses unsafe code to handle the self-referential pattern
/// required by `hnsw_rs`. The `HnswIo::load_hnsw()` returns an `Hnsw<'a>`
/// that borrows from `HnswIo`, but we need both to live in the same struct.
///
/// The safety is guaranteed by the caller storing `io_holder` in the struct
/// and ensuring proper drop order.
///
/// # Errors
///
/// Returns an error if:
/// - Mappings file is not found
/// - HNSW graph loading fails
/// - ID mappings deserialization fails
// RF-2.3: Will be used by HnswIndex::load after refactoring
#[allow(dead_code)]
pub(super) fn load_index(path: &Path, metric: DistanceMetric) -> std::io::Result<LoadedIndex> {
    let basename = "hnsw_index";

    // Check mappings file existence
    let mappings_path = path.join("id_mappings.bin");
    if !mappings_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "ID mappings file not found",
        ));
    }

    // 1. Load HNSW graph
    let mut io_holder = Box::new(HnswIo::new(path, basename));

    // SAFETY: Lifetime Extension for Self-Referential Pattern
    //
    // We extend the lifetime from 'a (borrowed from io_holder) to 'static.
    // This is safe because the caller guarantees:
    //
    // 1. CONTAINMENT: Both io_holder and the Hnsw live inside HnswIndex.
    // 2. DROP ORDER: HnswIndex::Drop drops inner BEFORE io_holder.
    // 3. NO ESCAPE: The 'static lifetime never escapes the struct.
    let io_ref: &'static mut HnswIo =
        unsafe { &mut *std::ptr::from_mut::<HnswIo>(io_holder.as_mut()) };

    let inner = match metric {
        DistanceMetric::Cosine => {
            let hnsw = io_ref
                .load_hnsw::<f32, DistCosine>()
                .map_err(std::io::Error::other)?;
            HnswInner::Cosine(hnsw)
        }
        DistanceMetric::Euclidean => {
            let hnsw = io_ref
                .load_hnsw::<f32, DistL2>()
                .map_err(std::io::Error::other)?;
            HnswInner::Euclidean(hnsw)
        }
        DistanceMetric::DotProduct => {
            let hnsw = io_ref
                .load_hnsw::<f32, DistDot>()
                .map_err(std::io::Error::other)?;
            HnswInner::DotProduct(hnsw)
        }
        DistanceMetric::Hamming => {
            let hnsw = io_ref
                .load_hnsw::<f32, DistL2>()
                .map_err(std::io::Error::other)?;
            HnswInner::Hamming(hnsw)
        }
        DistanceMetric::Jaccard => {
            let hnsw = io_ref
                .load_hnsw::<f32, DistL2>()
                .map_err(std::io::Error::other)?;
            HnswInner::Jaccard(hnsw)
        }
    };

    // 2. Load Mappings
    let file = std::fs::File::open(mappings_path)?;
    let reader = std::io::BufReader::new(file);
    let (id_to_idx, idx_to_id, next_idx): (HashMap<u64, usize>, HashMap<usize, u64>, usize) =
        bincode::deserialize_from(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    Ok(LoadedIndex {
        inner,
        mappings: ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx),
        io_holder,
    })
}

// ============================================================================
// Tests (TDD - written BEFORE refactoring index.rs)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Test that `save_index` creates expected files
    #[test]
    fn test_save_creates_files() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Create test data with at least one vector (hnsw_rs requirement)
        let hnsw = Hnsw::new(16, 100, 16, 200, DistCosine);
        hnsw.insert((&[0.1_f32, 0.2, 0.3, 0.4], 0));
        let inner = HnswInner::Cosine(hnsw);
        let inner_lock = RwLock::new(ManuallyDrop::new(inner));
        let mappings = ShardedMappings::new();

        // Act
        let result = save_index(path, &inner_lock, &mappings);

        // Assert
        assert!(
            result.is_ok(),
            "save_index failed: {err:?}",
            err = result.err()
        );
        assert!(path.join("hnsw_index.hnsw.data").exists());
        assert!(path.join("hnsw_index.hnsw.graph").exists());
        assert!(path.join("id_mappings.bin").exists());
    }

    /// Test that `load_index` returns error for missing files
    #[test]
    fn test_load_missing_mappings_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Act - try to load from empty directory
        let result = load_index(path, DistanceMetric::Cosine);

        // Assert
        match result {
            Err(err) => assert_eq!(err.kind(), std::io::ErrorKind::NotFound),
            Ok(_) => panic!("Expected error but got Ok"),
        }
    }

    /// Test save then load roundtrip preserves mappings
    #[test]
    fn test_save_load_roundtrip_preserves_mappings() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Create test data with vectors (hnsw_rs requirement)
        let hnsw = Hnsw::new(16, 100, 16, 200, DistCosine);
        hnsw.insert((&[0.1_f32, 0.2, 0.3, 0.4], 0));
        hnsw.insert((&[0.2_f32, 0.3, 0.4, 0.5], 1));
        hnsw.insert((&[0.3_f32, 0.4, 0.5, 0.6], 2));
        let inner = HnswInner::Cosine(hnsw);
        let inner_lock = RwLock::new(ManuallyDrop::new(inner));
        let mappings = ShardedMappings::new();

        // Register some IDs
        mappings.register(100);
        mappings.register(200);
        mappings.register(300);

        // Save
        save_index(path, &inner_lock, &mappings).expect("Failed to save");

        // Load
        let loaded = load_index(path, DistanceMetric::Cosine).expect("Failed to load index");

        // Assert mappings preserved
        assert_eq!(loaded.mappings.len(), 3);
        assert!(loaded.mappings.get_idx(100).is_some());
        assert!(loaded.mappings.get_idx(200).is_some());
        assert!(loaded.mappings.get_idx(300).is_some());
    }

    /// Test load works for all distance metrics
    #[test]
    #[allow(clippy::match_same_arms)]
    fn test_save_load_all_metrics() {
        let metrics = [
            DistanceMetric::Cosine,
            DistanceMetric::Euclidean,
            DistanceMetric::DotProduct,
            DistanceMetric::Hamming,
            DistanceMetric::Jaccard,
        ];

        let test_vector: [f32; 4] = [0.1, 0.2, 0.3, 0.4];

        for metric in metrics {
            let temp_dir = TempDir::new().unwrap();
            let path = temp_dir.path();

            // Create index with specific metric and insert vector
            let inner = match metric {
                DistanceMetric::Cosine => {
                    let hnsw = Hnsw::new(16, 100, 16, 200, DistCosine);
                    hnsw.insert((&test_vector, 0));
                    HnswInner::Cosine(hnsw)
                }
                DistanceMetric::Euclidean | DistanceMetric::Hamming | DistanceMetric::Jaccard => {
                    let hnsw = Hnsw::new(16, 100, 16, 200, DistL2);
                    hnsw.insert((&test_vector, 0));
                    HnswInner::Euclidean(hnsw)
                }
                DistanceMetric::DotProduct => {
                    let hnsw = Hnsw::new(16, 100, 16, 200, DistDot);
                    hnsw.insert((&test_vector, 0));
                    HnswInner::DotProduct(hnsw)
                }
            };
            let inner_lock = RwLock::new(ManuallyDrop::new(inner));
            let mappings = ShardedMappings::new();

            // Save and load
            save_index(path, &inner_lock, &mappings)
                .unwrap_or_else(|e| panic!("Save failed for {metric:?}: {e:?}"));
            let result = load_index(path, metric);

            assert!(result.is_ok(), "Load failed for metric {metric:?}");
        }
    }
}
