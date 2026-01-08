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
