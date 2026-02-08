//! Shared HNSW persistence helpers for metadata and mappings serialization.
//!
//! Consolidates duplicated bincode save/load logic used by both `HnswIndex`
//! and `NativeHnswIndex` to prevent format drift between the two index types.
//!
//! # On-Disk Format
//!
//! Both index types share the same binary format:
//! - `native_meta.bin`: `(dimension: usize, metric: u8, enable_vector_storage: bool)`
//! - `native_mappings.bin`: `(id_to_idx: HashMap<u64, usize>, idx_to_id: HashMap<usize, u64>, next_idx: usize)`

use crate::distance::DistanceMetric;
use std::collections::HashMap;
use std::path::Path;

/// HNSW index metadata as stored on disk.
pub(crate) struct HnswMeta {
    pub dimension: usize,
    pub metric: DistanceMetric,
    pub enable_vector_storage: bool,
}

/// HNSW mappings data as stored on disk.
pub(crate) struct HnswMappingsData {
    pub id_to_idx: HashMap<u64, usize>,
    pub idx_to_id: HashMap<usize, u64>,
    pub next_idx: usize,
}

/// Saves HNSW metadata to `native_meta.bin` in the given directory.
///
/// # Errors
///
/// Returns `io::Error` if file creation or serialization fails.
pub(crate) fn save_meta(path: &Path, meta: &HnswMeta) -> std::io::Result<()> {
    let meta_path = path.join("native_meta.bin");
    let meta_file = std::fs::File::create(meta_path)?;
    let meta_writer = std::io::BufWriter::new(meta_file);
    bincode::serialize_into(
        meta_writer,
        &(
            meta.dimension,
            meta.metric as u8,
            meta.enable_vector_storage,
        ),
    )
    .map_err(std::io::Error::other)
}

/// Loads HNSW metadata from `native_meta.bin` in the given directory.
///
/// # Errors
///
/// Returns `io::Error` if the file doesn't exist, is corrupted, or
/// contains an unknown metric discriminant.
pub(crate) fn load_meta(path: &Path) -> std::io::Result<HnswMeta> {
    let meta_path = path.join("native_meta.bin");
    let meta_file = std::fs::File::open(meta_path)?;
    let meta_reader = std::io::BufReader::new(meta_file);
    let (dimension, metric_u8, enable_vector_storage): (usize, u8, bool) =
        bincode::deserialize_from(meta_reader).map_err(std::io::Error::other)?;

    // Match enum order: Cosine=0, Euclidean=1, DotProduct=2, Hamming=3, Jaccard=4
    let metric = metric_from_u8(metric_u8)?;

    Ok(HnswMeta {
        dimension,
        metric,
        enable_vector_storage,
    })
}

/// Saves HNSW id-mappings to `native_mappings.bin` in the given directory.
///
/// # Errors
///
/// Returns `io::Error` if file creation or serialization fails.
pub(crate) fn save_mappings(path: &Path, data: &HnswMappingsData) -> std::io::Result<()> {
    let mappings_path = path.join("native_mappings.bin");
    let file = std::fs::File::create(mappings_path)?;
    let writer = std::io::BufWriter::new(file);
    bincode::serialize_into(writer, &(&data.id_to_idx, &data.idx_to_id, data.next_idx))
        .map_err(std::io::Error::other)
}

/// Loads HNSW id-mappings from `native_mappings.bin` in the given directory.
///
/// # Errors
///
/// Returns `io::Error` if the file doesn't exist or is corrupted.
pub(crate) fn load_mappings(path: &Path) -> std::io::Result<HnswMappingsData> {
    let mappings_path = path.join("native_mappings.bin");
    let file = std::fs::File::open(mappings_path)?;
    let reader = std::io::BufReader::new(file);

    let (id_to_idx, idx_to_id, next_idx): (HashMap<u64, usize>, HashMap<usize, u64>, usize) =
        bincode::deserialize_from(reader).map_err(std::io::Error::other)?;

    Ok(HnswMappingsData {
        id_to_idx,
        idx_to_id,
        next_idx,
    })
}

/// Converts a u8 discriminant to a `DistanceMetric`.
///
/// # Errors
///
/// Returns `io::Error` with `InvalidData` kind if the discriminant is unknown.
fn metric_from_u8(value: u8) -> std::io::Result<DistanceMetric> {
    match value {
        0 => Ok(DistanceMetric::Cosine),
        1 => Ok(DistanceMetric::Euclidean),
        2 => Ok(DistanceMetric::DotProduct),
        3 => Ok(DistanceMetric::Hamming),
        4 => Ok(DistanceMetric::Jaccard),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Unknown distance metric",
        )),
    }
}
