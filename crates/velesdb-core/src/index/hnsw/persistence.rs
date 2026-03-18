//! Shared HNSW persistence helpers for metadata and mappings serialization.
//!
//! Consolidates duplicated postcard save/load logic used by both `HnswIndex`
//! and `NativeHnswIndex` to prevent format drift between the two index types.
//!
//! # On-Disk Format
//!
//! Both index types share the same binary format:
//! - `native_meta.bin`: `(dimension: usize, metric: u8, enable_vector_storage: bool)`
//! - `native_mappings.bin`: `(id_to_idx: HashMap<u64, usize>, idx_to_id: HashMap<usize, u64>, next_idx: usize)`
//! - `native_vectors.bin`: `Vec<(internal_idx: usize, vector: Vec<f32>)>`

use crate::distance::DistanceMetric;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// HNSW vectors payload as stored on disk.
pub(crate) struct HnswVectorsData {
    pub vectors: Vec<(usize, Vec<f32>)>,
}

/// Saves HNSW metadata to `native_meta.bin` in the given directory.
///
/// Uses atomic write-tmp-fsync-rename to prevent torn writes on crash.
///
/// # Errors
///
/// Returns `io::Error` if file creation or serialization fails.
pub(crate) fn save_meta(path: &Path, meta: &HnswMeta) -> std::io::Result<()> {
    let meta_path = path.join("native_meta.bin");
    let bytes = postcard::to_allocvec(&(
        meta.dimension,
        meta.metric as u8,
        meta.enable_vector_storage,
    ))
    .map_err(std::io::Error::other)?;
    atomic_write(&meta_path, &bytes)
}

/// Loads HNSW metadata from `native_meta.bin` in the given directory.
///
/// # Errors
///
/// Returns `io::Error` if the file doesn't exist, is corrupted, or
/// contains an unknown metric discriminant.
pub(crate) fn load_meta(path: &Path) -> std::io::Result<HnswMeta> {
    let meta_path = path.join("native_meta.bin");
    let bytes = std::fs::read(meta_path)?;
    let (dimension, metric_u8, enable_vector_storage): (usize, u8, bool) =
        postcard::from_bytes(&bytes).map_err(std::io::Error::other)?;

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
/// Uses atomic write-tmp-fsync-rename to prevent torn writes on crash.
///
/// # Errors
///
/// Returns `io::Error` if file creation or serialization fails.
pub(crate) fn save_mappings(path: &Path, data: &HnswMappingsData) -> std::io::Result<()> {
    let mappings_path = path.join("native_mappings.bin");
    let bytes = postcard::to_allocvec(&(&data.id_to_idx, &data.idx_to_id, data.next_idx))
        .map_err(std::io::Error::other)?;
    atomic_write(&mappings_path, &bytes)
}

/// Loads HNSW id-mappings from `native_mappings.bin` in the given directory.
///
/// # Errors
///
/// Returns `io::Error` if the file doesn't exist or is corrupted.
pub(crate) fn load_mappings(path: &Path) -> std::io::Result<HnswMappingsData> {
    let mappings_path = path.join("native_mappings.bin");
    let bytes = std::fs::read(mappings_path)?;

    let (id_to_idx, idx_to_id, next_idx): (HashMap<u64, usize>, HashMap<usize, u64>, usize) =
        postcard::from_bytes(&bytes).map_err(std::io::Error::other)?;

    Ok(HnswMappingsData {
        id_to_idx,
        idx_to_id,
        next_idx,
    })
}

/// Saves HNSW vectors to `native_vectors.bin` in the given directory.
///
/// Uses atomic write-tmp-fsync-rename to prevent torn writes on crash.
///
/// # Errors
///
/// Returns `io::Error` if file creation or serialization fails.
pub(crate) fn save_vectors(path: &Path, data: &HnswVectorsData) -> std::io::Result<()> {
    let vectors_path = path.join("native_vectors.bin");
    let bytes = postcard::to_allocvec(&data.vectors).map_err(std::io::Error::other)?;
    atomic_write(&vectors_path, &bytes)
}

/// Loads HNSW vectors from `native_vectors.bin` in the given directory.
///
/// # Errors
///
/// Returns `io::Error` if the file doesn't exist or is corrupted.
pub(crate) fn load_vectors(path: &Path) -> std::io::Result<HnswVectorsData> {
    let vectors_path = path.join("native_vectors.bin");
    let bytes = std::fs::read(vectors_path)?;
    let vectors: Vec<(usize, Vec<f32>)> =
        postcard::from_bytes(&bytes).map_err(std::io::Error::other)?;
    Ok(HnswVectorsData { vectors })
}

/// Writes `data` to a unique temp file, fsyncs, then renames to `final_path`.
///
/// This provides crash-safe persistence: readers always see either the
/// previous complete file or the new complete file, never a torn write.
///
/// Each call generates a unique temporary filename using process ID, thread ID,
/// and a global counter to prevent races both within a process (concurrent
/// threads) and across processes sharing the same data directory.
fn atomic_write(final_path: &Path, data: &[u8]) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let tid = std::thread::current().id();

    // Build temp file in the same directory as the target, with a unique suffix
    // derived from PID + thread ID + global counter to avoid concurrent-write
    // races both intra-process and cross-process.
    let file_name = final_path.file_name().unwrap_or_default().to_string_lossy();
    let tmp_name = format!("{file_name}.tmp.{pid}.{tid:?}.{seq}");
    let tmp_path = final_path.with_file_name(&tmp_name);

    let result = atomic_write_inner(&tmp_path, final_path, data);
    if result.is_err() {
        // Best-effort cleanup of the temp file on failure.
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

/// Inner write-fsync-rename step for [`atomic_write`].
fn atomic_write_inner(tmp_path: &Path, final_path: &Path, data: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(tmp_path)?;
    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(data)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    std::fs::rename(tmp_path, final_path)
}

/// Loads vectors from disk, disabling vector storage gracefully when the file
/// is missing (e.g., index was saved in fast-insert mode before vectors existed).
///
/// RF-DEDUP: This pattern was duplicated in `HnswIndex::load` and
/// `NativeHnswIndex::load`. Now both delegate here.
///
/// # Errors
///
/// Returns `io::Error` if the vectors file exists but cannot be read/deserialized.
pub(crate) fn load_vectors_or_disable(
    path: &Path,
    meta: &HnswMeta,
) -> std::io::Result<(super::sharded_vectors::ShardedVectors, bool)> {
    use super::sharded_vectors::ShardedVectors;

    if !meta.enable_vector_storage {
        return Ok((ShardedVectors::new(meta.dimension), false));
    }

    match load_vectors(path) {
        Ok(vectors_data) => {
            let vectors = ShardedVectors::new(meta.dimension);
            vectors.insert_batch(vectors_data.vectors);
            Ok((vectors, true))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                "native_vectors.bin missing during HNSW load; disabling vector storage for safety"
            );
            Ok((ShardedVectors::new(meta.dimension), false))
        }
        Err(err) => Err(err),
    }
}

/// Persists vectors to disk or removes stale vector files.
///
/// RF-DEDUP: This pattern was duplicated in `HnswIndex::save` and
/// `NativeHnswIndex::save`. Now both delegate here.
///
/// # Errors
///
/// Returns `io::Error` if the file operation fails.
pub(crate) fn save_or_cleanup_vectors(
    path: &Path,
    enable_vector_storage: bool,
    vectors: &super::sharded_vectors::ShardedVectors,
) -> std::io::Result<()> {
    if enable_vector_storage {
        save_vectors(
            path,
            &HnswVectorsData {
                vectors: vectors.collect_for_parallel(),
            },
        )
    } else {
        let vectors_path = path.join("native_vectors.bin");
        if vectors_path.exists() {
            std::fs::remove_file(vectors_path)?;
        }
        Ok(())
    }
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
