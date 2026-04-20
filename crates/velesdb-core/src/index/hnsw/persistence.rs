//! Shared HNSW persistence helpers for metadata and mappings serialization.
//!
//! Consolidates duplicated postcard save/load logic used by both `HnswIndex`
//! and `NativeHnswIndex` to prevent format drift between the two index types.
//!
//! # On-Disk Format
//!
//! Both index types share the same binary format. Every sidecar carries a
//! `generation: u64` stamp so that [`load_sidecars`] can detect a partial
//! save (see issue #617 — the 3-file sequence is not itself atomic even
//! though each individual file is written atomically).
//!
//! - `native_meta.bin`: 5-tuple `(dimension: usize, metric: u8,
//!   enable_vector_storage: bool, storage_mode: u8, generation: u64)`.
//!   Backward-compat: 4-tuple (v1.7.2+, generation=0) and 3-tuple
//!   (pre-v1.7.2, generation=0, `storage_mode=Full`) are still accepted on
//!   load.
//! - `native_mappings.bin`: 4-tuple `(id_to_idx: HashMap<u64, usize>,
//!   idx_to_id: HashMap<usize, u64>, next_idx: usize, generation: u64)`.
//!   Backward-compat: 3-tuple (generation=0) accepted on load.
//! - `native_vectors.bin`: 2-tuple `(Vec<(internal_idx: usize, vector:
//!   Vec<f32>)>, generation: u64)`. Backward-compat: plain `Vec`
//!   (generation=0) accepted on load.

use crate::distance::DistanceMetric;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// HNSW index metadata as stored on disk.
///
/// The `generation` field is the authoritative commit stamp used to
/// detect partial `save_sidecars` writes (see [`save_sidecars`] and
/// issue #617). `meta` is written last during save, so its generation is
/// the ground truth that the other two sidecars must match on load.
pub(crate) struct HnswMeta {
    pub dimension: usize,
    pub metric: DistanceMetric,
    pub enable_vector_storage: bool,
    /// Storage mode for the HNSW backend (defaults to `Full` for backward compat).
    pub storage_mode: crate::StorageMode,
    /// Monotonic save generation. `0` for DBs written by pre-fix binaries.
    pub generation: u64,
}

/// HNSW mappings data as stored on disk.
pub(crate) struct HnswMappingsData {
    pub id_to_idx: HashMap<u64, usize>,
    pub idx_to_id: HashMap<usize, u64>,
    pub next_idx: usize,
    /// Must match [`HnswMeta::generation`] on load — mismatch = partial save.
    pub generation: u64,
}

/// HNSW vectors payload as stored on disk.
pub(crate) struct HnswVectorsData {
    pub vectors: Vec<(usize, Vec<f32>)>,
    /// Must match [`HnswMeta::generation`] on load — mismatch = partial save.
    pub generation: u64,
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
        storage_mode_to_u8(meta.storage_mode),
        meta.generation,
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

    // Try 5-tuple (post-#617, with generation) first.
    if let Ok((dimension, metric_u8, enable_vector_storage, storage_mode_u8, generation)) =
        postcard::from_bytes::<(usize, u8, bool, u8, u64)>(&bytes)
    {
        let metric = metric_from_u8(metric_u8)?;
        let storage_mode = storage_mode_from_u8(storage_mode_u8);
        return Ok(HnswMeta {
            dimension,
            metric,
            enable_vector_storage,
            storage_mode,
            generation,
        });
    }

    // Backward-compat: 4-tuple format (v1.7.2+) — no generation stamp.
    if let Ok((dimension, metric_u8, enable_vector_storage, storage_mode_u8)) =
        postcard::from_bytes::<(usize, u8, bool, u8)>(&bytes)
    {
        let metric = metric_from_u8(metric_u8)?;
        let storage_mode = storage_mode_from_u8(storage_mode_u8);
        return Ok(HnswMeta {
            dimension,
            metric,
            enable_vector_storage,
            storage_mode,
            generation: 0,
        });
    }

    // Backward-compat: 3-tuple format (pre-v1.7.2) defaults to Full.
    let (dimension, metric_u8, enable_vector_storage): (usize, u8, bool) =
        postcard::from_bytes(&bytes).map_err(std::io::Error::other)?;
    let metric = metric_from_u8(metric_u8)?;

    Ok(HnswMeta {
        dimension,
        metric,
        enable_vector_storage,
        storage_mode: crate::StorageMode::Full,
        generation: 0,
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
    let bytes = postcard::to_allocvec(&(
        &data.id_to_idx,
        &data.idx_to_id,
        data.next_idx,
        data.generation,
    ))
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

    // Try 4-tuple (post-#617, with generation) first.
    if let Ok((id_to_idx, idx_to_id, next_idx, generation)) =
        postcard::from_bytes::<(HashMap<u64, usize>, HashMap<usize, u64>, usize, u64)>(&bytes)
    {
        return Ok(HnswMappingsData {
            id_to_idx,
            idx_to_id,
            next_idx,
            generation,
        });
    }

    // Backward-compat: 3-tuple format (pre-#617) — no generation stamp.
    let (id_to_idx, idx_to_id, next_idx): (HashMap<u64, usize>, HashMap<usize, u64>, usize) =
        postcard::from_bytes(&bytes).map_err(std::io::Error::other)?;

    Ok(HnswMappingsData {
        id_to_idx,
        idx_to_id,
        next_idx,
        generation: 0,
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
    let bytes =
        postcard::to_allocvec(&(&data.vectors, data.generation)).map_err(std::io::Error::other)?;
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

    // Try 2-tuple (post-#617, with generation) first.
    if let Ok((vectors, generation)) = postcard::from_bytes::<(Vec<(usize, Vec<f32>)>, u64)>(&bytes)
    {
        return Ok(HnswVectorsData {
            vectors,
            generation,
        });
    }

    // Backward-compat: plain `Vec` payload (pre-#617) — no generation stamp.
    let vectors: Vec<(usize, Vec<f32>)> =
        postcard::from_bytes(&bytes).map_err(std::io::Error::other)?;
    Ok(HnswVectorsData {
        vectors,
        generation: 0,
    })
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
/// Returns `(vectors, enable_vector_storage, vectors_generation)`. The
/// third element is the on-disk generation stamp (see #617); it is only
/// meaningful when `enable_vector_storage` is `true`. When vectors are
/// disabled or the file is absent, the returned generation is `0`.
///
/// # Errors
///
/// Returns `io::Error` if the vectors file exists but cannot be read/deserialized.
pub(crate) fn load_vectors_or_disable(
    path: &Path,
    meta: &HnswMeta,
) -> std::io::Result<(super::sharded_vectors::ShardedVectors, bool, u64)> {
    use super::sharded_vectors::ShardedVectors;

    if !meta.enable_vector_storage {
        return Ok((ShardedVectors::new(meta.dimension), false, 0));
    }

    match load_vectors(path) {
        Ok(vectors_data) => {
            let vectors = ShardedVectors::new(meta.dimension);
            vectors.insert_batch(vectors_data.vectors);
            Ok((vectors, true, vectors_data.generation))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                "native_vectors.bin missing during HNSW load; disabling vector storage for safety"
            );
            Ok((ShardedVectors::new(meta.dimension), false, 0))
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
    generation: u64,
) -> std::io::Result<()> {
    if enable_vector_storage {
        save_vectors(
            path,
            &HnswVectorsData {
                vectors: vectors.collect_for_parallel(),
                generation,
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

/// Reads the current on-disk generation from `native_meta.bin`, if any.
///
/// Returns `None` when meta is missing or unreadable — the caller then
/// treats the directory as fresh and starts at generation 0. Legacy DBs
/// (pre-#617 format) return `Some(0)` via [`load_meta`]'s backward-compat
/// fallback, so the next save lands at generation 1 regardless.
fn read_current_generation(path: &Path) -> Option<u64> {
    load_meta(path).ok().map(|meta| meta.generation)
}

/// Returns the generation number to stamp on the next save at `path`.
///
/// Callers that write artefacts outside the sidecar trio (e.g. the HNSW
/// graph file via `file_dump`) must call this once, then pass the returned
/// value to both [`save_graph_generation`] and [`save_sidecars`] so every
/// artefact is stamped with the same monotonic counter.
pub(crate) fn next_generation(path: &Path) -> u64 {
    read_current_generation(path).unwrap_or(0).saturating_add(1)
}

/// Writes the HNSW graph generation marker (`native_hnsw.gen`) atomically.
///
/// Complements the graph file (`native_hnsw`) dumped by the caller before
/// invoking [`save_sidecars`]. Closes the atomicity gap between graph dump
/// and sidecar writes (issue #617 Devin follow-up): any crash after graph
/// dump but before sidecar writes leaves `native_hnsw.gen` at the new
/// generation while the sidecars remain at the old one, so [`load_sidecars`]
/// detects the mismatch.
///
/// # Errors
///
/// Returns `io::Error` if serialization or the atomic write fails.
pub(crate) fn save_graph_generation(path: &Path, generation: u64) -> std::io::Result<()> {
    let marker_path = path.join("native_hnsw.gen");
    let bytes = postcard::to_allocvec(&generation).map_err(std::io::Error::other)?;
    atomic_write(&marker_path, &bytes)
}

/// Reads the HNSW graph generation marker, returning `0` when the file is
/// missing for backward compatibility with pre-#617 databases.
///
/// Pre-#617 saves did not write `native_hnsw.gen`, so legacy DBs carry gen 0
/// everywhere — the consistency check in [`load_sidecars`] passes trivially
/// (0 == 0 == 0 == 0).
///
/// # Errors
///
/// Returns `io::Error` only when the file exists but is unreadable /
/// corrupt. A missing file is not an error (returns `Ok(0)`).
pub(crate) fn load_graph_generation(path: &Path) -> std::io::Result<u64> {
    let marker_path = path.join("native_hnsw.gen");
    match std::fs::read(&marker_path) {
        Ok(bytes) => postcard::from_bytes::<u64>(&bytes).map_err(std::io::Error::other),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err),
    }
}

/// Persists every non-graph sidecar (mappings, vectors, meta) for an HNSW
/// index in one call.
///
/// Both `HnswIndex::save` and `NativeHnswIndex::save` need the same 3-step
/// sidecar sequence after they dump the graph. Consolidating it here removes
/// format drift risk (the two call sites previously had identical code but
/// could silently diverge on the next field addition to `HnswMeta`).
///
/// The HNSW graph itself is dumped by the caller, because the two index
/// types use different inner types (`NativeHnswInner` directly vs
/// `ManuallyDrop<HnswInner>`) that would otherwise require a trait object.
///
/// # Atomicity (issue #617)
///
/// Individual file writes are atomic (via [`atomic_write`]), but the 3-file
/// sequence is not — a crash between two renames leaves the on-disk state
/// inconsistent. To detect such a crash on reload, every sidecar is stamped
/// with the same monotonic `new_gen: u64`, computed by the caller via
/// [`next_generation`] BEFORE dumping the HNSW graph. `meta` is written
/// LAST: its generation is the authoritative commit point. On load,
/// [`load_sidecars`] verifies that mappings, vectors, and the graph marker
/// all carry the same generation as meta — any mismatch is reported as
/// `InvalidData`.
///
/// The caller must pre-compute `new_gen = next_generation(path)` once and
/// pass the same value to [`save_graph_generation`] and this function, so
/// the graph file and the sidecars land on the same generation stamp.
///
/// The caller-provided [`HnswMeta::generation`] is ignored; this function
/// overwrites it with `new_gen`.
///
/// # Errors
///
/// Returns `io::Error` if any of the three file operations fail.
pub(crate) fn save_sidecars(
    path: &Path,
    mappings: &super::sharded_mappings::ShardedMappings,
    vectors: &super::sharded_vectors::ShardedVectors,
    meta: &HnswMeta,
    new_gen: u64,
) -> std::io::Result<()> {
    let (id_to_idx, idx_to_id, next_idx) = mappings.as_parts();
    save_mappings(
        path,
        &HnswMappingsData {
            id_to_idx,
            idx_to_id,
            next_idx,
            generation: new_gen,
        },
    )?;
    save_or_cleanup_vectors(path, meta.enable_vector_storage, vectors, new_gen)?;

    // `meta` is written LAST — its generation is the authoritative commit
    // point that `load_sidecars` checks the other artefacts against.
    let stamped_meta = HnswMeta {
        generation: new_gen,
        ..*meta
    };
    save_meta(path, &stamped_meta)
}

/// Loads non-graph sidecars (mappings + vectors) for an HNSW index given a
/// previously loaded [`HnswMeta`].
///
/// Complements [`save_sidecars`]. The HNSW graph itself is loaded by the
/// caller (different inner types, see [`save_sidecars`]).
///
/// # Atomicity check (issue #617)
///
/// Every persisted artefact (graph marker, mappings, vectors) carries a
/// `generation: u64` stamp written by the pair
/// [`save_graph_generation`] + [`save_sidecars`]. `meta.generation` is the
/// authoritative commit point. If any artefact carries a stale or
/// mismatched generation, the database is proven to be in an inconsistent
/// state (crash between file renames during the previous save) and this
/// function returns `InvalidData` rather than silently loading a torn
/// state.
///
/// Legacy DBs written by pre-#617 binaries have generation=0 everywhere
/// and no `native_hnsw.gen` marker — [`load_graph_generation`] returns 0
/// for missing markers, so the consistency check passes trivially.
///
/// # Errors
///
/// Returns `io::Error::InvalidData` if the on-disk generations disagree
/// with `meta.generation`. Also returns `io::Error` if the mappings file
/// is missing or corrupt. Missing vectors files are tolerated and
/// gracefully disable vector storage — see [`load_vectors_or_disable`].
pub(crate) fn load_sidecars(
    path: &Path,
    meta: &HnswMeta,
) -> std::io::Result<(
    super::sharded_mappings::ShardedMappings,
    super::sharded_vectors::ShardedVectors,
    bool,
)> {
    let graph_generation = load_graph_generation(path)?;
    if graph_generation != meta.generation {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "incomplete save detected: graph generation {} but meta generation {} \
                 (crash between graph dump and sidecar writes, database state inconsistent)",
                graph_generation, meta.generation,
            ),
        ));
    }

    let mappings_data = load_mappings(path)?;
    if mappings_data.generation != meta.generation {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "incomplete save detected: mappings generation {} but meta generation {} \
                 (crash between sidecar writes, database state inconsistent)",
                mappings_data.generation, meta.generation,
            ),
        ));
    }

    let (vectors, enable_vector_storage, vectors_generation) = load_vectors_or_disable(path, meta)?;
    if enable_vector_storage && vectors_generation != meta.generation {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "incomplete save detected: vectors generation {} but meta generation {} \
                 (crash between sidecar writes, database state inconsistent)",
                vectors_generation, meta.generation,
            ),
        ));
    }

    let mappings = super::sharded_mappings::ShardedMappings::from_parts(
        mappings_data.id_to_idx,
        mappings_data.idx_to_id,
        mappings_data.next_idx,
    );
    Ok((mappings, vectors, enable_vector_storage))
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

/// Encodes [`StorageMode`] as a `u8` for on-disk persistence.
const fn storage_mode_to_u8(mode: crate::StorageMode) -> u8 {
    match mode {
        crate::StorageMode::Full => 0,
        crate::StorageMode::SQ8 => 1,
        crate::StorageMode::Binary => 2,
        crate::StorageMode::ProductQuantization => 3,
        crate::StorageMode::RaBitQ => 4,
    }
}

/// Decodes a `u8` from disk to [`StorageMode`], defaulting to `Full` for unknown values.
const fn storage_mode_from_u8(value: u8) -> crate::StorageMode {
    match value {
        1 => crate::StorageMode::SQ8,
        2 => crate::StorageMode::Binary,
        3 => crate::StorageMode::ProductQuantization,
        4 => crate::StorageMode::RaBitQ,
        // 0 and unknown values default to Full
        _ => crate::StorageMode::Full,
    }
}
