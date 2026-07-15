//! Shared HNSW persistence helpers for metadata and mappings serialization.
//!
//! Consolidates duplicated postcard save/load logic used by both `HnswIndex`
//! and `NativeHnswIndex` to prevent format drift between the two index types.
//!
//! # On-Disk Format
//!
//! Both index types share the same binary format. Every sidecar carries a
//! `generation: u64` stamp so that [`load_sidecars`] can detect a partial
//! save (see issue #617 — the multi-file sequence is not itself atomic even
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
//! - `native_vectors.bin`: **legacy only (PERF1)**. Older binaries persisted
//!   a duplicate of the vectors here (2-tuple `(Vec<(internal_idx: usize,
//!   vector: Vec<f32>)>, generation: u64)`; pre-#617 plain `Vec`,
//!   generation=0). The vectors now live solely in the graph's own
//!   `native_hnsw.vectors` file. On load, a legacy `native_vectors.bin` is
//!   still parsed for the #617 generation check and dimension validation,
//!   then its payload is discarded. [`save_sidecars`] deletes the file so
//!   a stale copy can never shadow newer graph data.

use crate::distance::DistanceMetric;
use crate::storage::atomic_write::atomic_write;
use std::collections::HashMap;
use std::path::Path;

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

/// Legacy HNSW vectors payload as stored on disk (`native_vectors.bin`).
///
/// New saves no longer write this file (PERF1 — the graph's
/// `native_hnsw.vectors` is the single vector store). Kept for reading
/// databases written by older binaries: the generation stamp still
/// participates in the #617 consistency check, the payload is discarded.
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

/// Writes a legacy-format `native_vectors.bin` (test-only).
///
/// Production code no longer persists this file (PERF1); this writer is
/// kept so tests can build fixtures that mimic databases written by older
/// binaries and exercise the legacy-read path of [`load_sidecars`].
#[cfg(test)]
pub(crate) fn save_vectors(path: &Path, data: &HnswVectorsData) -> std::io::Result<()> {
    let vectors_path = path.join("native_vectors.bin");
    let bytes =
        postcard::to_allocvec(&(&data.vectors, data.generation)).map_err(std::io::Error::other)?;
    atomic_write(&vectors_path, &bytes)
}

/// Loads legacy HNSW vectors from `native_vectors.bin` in the given directory.
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

// Crash-safe persistence (write-tmp + fsync + rename) is provided by the shared
// `crate::storage::atomic_write` helper, imported above.

/// Checks a legacy `native_vectors.bin` left by an older binary, then
/// discards its payload (PERF1 — the graph's `native_hnsw.vectors` is the
/// single vector store).
///
/// When the file is present it must still be internally valid (dimension,
/// unique indices) and carry the meta generation, otherwise the on-disk
/// state is torn (crash mid-save under the old 4-file scheme) and loading
/// must fail exactly as it did before the sidecar removal. A missing file
/// is the normal case for databases saved by current binaries.
///
/// # Errors
///
/// Returns `InvalidData` if the file exists but is corrupted, has wrong
/// dimensions, or carries a generation different from `meta.generation`.
fn check_legacy_vectors_file(path: &Path, meta: &HnswMeta) -> std::io::Result<()> {
    match load_vectors(path) {
        Ok(vectors_data) => {
            validate_loaded_vectors(&vectors_data.vectors, meta.dimension)?;
            check_sidecar_generation(
                "vectors",
                vectors_data.generation,
                meta.generation,
                "crash between sidecar writes",
            )
            // Payload intentionally dropped: vectors were loaded from the
            // graph's own file before this function ran.
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Removes a stale legacy `native_vectors.bin`, if any.
///
/// Called on every save so a file written by an older binary can never
/// outlive the snapshot it belonged to (it would otherwise fail the #617
/// generation check on the next load).
fn cleanup_legacy_vectors_file(path: &Path) -> std::io::Result<()> {
    let vectors_path = path.join("native_vectors.bin");
    if vectors_path.exists() {
        std::fs::remove_file(vectors_path)?;
    }
    Ok(())
}

/// Reads the current on-disk generation from `native_meta.bin` with
/// fail-fast semantics on real I/O or corruption errors.
///
/// Returns:
/// - `Ok(Some(gen))` when meta exists and was parseable (incl. legacy
///   backward-compat fallbacks, which yield `gen=0`).
/// - `Ok(None)` only when meta does NOT exist — the caller then treats
///   the directory as fresh and starts at generation 1.
/// - `Err(err)` on any other I/O or deserialization failure (corrupted
///   meta, permission denied, etc.). Callers MUST NOT paper over this —
///   proceeding with a fresh generation=1 save would overwrite potentially
///   recoverable corrupted state (Devin #618 follow-up).
fn read_current_generation(path: &Path) -> std::io::Result<Option<u64>> {
    match load_meta(path) {
        Ok(meta) => Ok(Some(meta.generation)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Returns the generation number to stamp on the next save at `path`.
///
/// Callers that write artefacts outside the sidecar trio (e.g. the HNSW
/// graph file via `file_dump`) must call this once, then pass the returned
/// value to both [`save_graph_generation`] and [`save_sidecars`] so every
/// artefact is stamped with the same monotonic counter.
///
/// # Errors
///
/// Returns `io::Error` when meta exists but is unreadable or corrupted —
/// the caller must propagate rather than silently starting at generation
/// 1, which would overwrite potentially recoverable state (Devin #618
/// follow-up). A missing meta is not an error (returns `Ok(1)` for fresh
/// directories).
pub(crate) fn next_generation(path: &Path) -> std::io::Result<u64> {
    Ok(read_current_generation(path)?
        .unwrap_or(0)
        .saturating_add(1))
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

/// Persists every non-graph sidecar (mappings, meta) for an HNSW index in
/// one call.
///
/// Both `HnswIndex::save` and `NativeHnswIndex::save` need the same sidecar
/// sequence after they dump the graph. Consolidating it here removes format
/// drift risk (the two call sites previously had identical code but could
/// silently diverge on the next field addition to `HnswMeta`).
///
/// The HNSW graph itself is dumped by the caller, because the two index
/// types use different inner types (`NativeHnswInner` directly vs
/// `ManuallyDrop<HnswInner>`) that would otherwise require a trait object.
/// Vector data lives inside the graph dump (`native_hnsw.vectors`) — the
/// legacy `native_vectors.bin` duplicate is no longer written and any stale
/// copy from an older binary is deleted here (PERF1).
///
/// # Atomicity (issue #617)
///
/// Individual file writes are atomic (via [`atomic_write`]), but the
/// multi-file sequence is not — a crash between two renames leaves the
/// on-disk state inconsistent. To detect such a crash on reload, every
/// sidecar is stamped with the same monotonic `new_gen: u64`, computed by
/// the caller via [`next_generation`] BEFORE dumping the HNSW graph. `meta`
/// is written LAST: its generation is the authoritative commit point. On
/// load, [`load_sidecars`] verifies that mappings and the graph marker
/// (which covers the graph dump, vectors included) carry the same
/// generation as meta — any mismatch is reported as `InvalidData`.
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
/// Returns `io::Error` if any of the file operations fail.
pub(crate) fn save_sidecars(
    path: &Path,
    mappings: &super::sharded_mappings::ShardedMappings,
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
    // A leftover legacy vectors file would carry an older generation and
    // fail the next load's consistency check — remove it eagerly.
    cleanup_legacy_vectors_file(path)?;

    // `meta` is written LAST — its generation is the authoritative commit
    // point that `load_sidecars` checks the other artefacts against.
    let stamped_meta = HnswMeta {
        generation: new_gen,
        ..*meta
    };
    save_meta(path, &stamped_meta)
}

/// Loads the non-graph sidecar (mappings) for an HNSW index given a
/// previously loaded [`HnswMeta`] and the vector count of the
/// already-loaded graph.
///
/// Complements [`save_sidecars`]. The HNSW graph itself is loaded by the
/// caller (different inner types, see [`save_sidecars`]) — `graph_vector_count`
/// is the number of vector slots in its `ContiguousVectors`, which is the
/// store every mapped internal index resolves into at runtime.
///
/// # Atomicity check (issue #617)
///
/// Every persisted artefact (graph marker, mappings — plus the legacy
/// vectors file when present) carries a `generation: u64` stamp written by
/// the pair [`save_graph_generation`] + [`save_sidecars`].
/// `meta.generation` is the authoritative commit point. If any artefact
/// carries a stale or mismatched generation, the database is proven to be
/// in an inconsistent state (crash between file renames during the
/// previous save) and this function returns `InvalidData` rather than
/// silently loading a torn state.
///
/// The check went from 4 artefacts to 3 with the sidecar removal (PERF1):
/// the vector payload now lives inside the graph dump, whose own
/// `native_hnsw.gen` marker already covers it — the old vectors leg was a
/// duplicate of the graph leg, not an independent signal. Databases
/// written by older binaries still have `native_vectors.bin`; its stamp is
/// verified (4-leg check preserved) and its payload discarded.
///
/// Legacy DBs written by pre-#617 binaries have generation=0 everywhere
/// and no `native_hnsw.gen` marker — [`load_graph_generation`] returns 0
/// for missing markers, so the consistency check passes trivially.
///
/// # Errors
///
/// Returns `io::Error::InvalidData` if the on-disk generations disagree
/// with `meta.generation`, or if any mapped index falls outside the loaded
/// graph's vector store. Also returns `io::Error` if the mappings file is
/// missing or corrupt.
pub(crate) fn load_sidecars(
    path: &Path,
    meta: &HnswMeta,
    graph_vector_count: usize,
) -> std::io::Result<super::sharded_mappings::ShardedMappings> {
    let graph_generation = load_graph_generation(path)?;
    check_sidecar_generation(
        "graph",
        graph_generation,
        meta.generation,
        "crash between graph dump and sidecar writes",
    )?;

    let mappings_data = load_mappings(path)?;
    check_sidecar_generation(
        "mappings",
        mappings_data.generation,
        meta.generation,
        "crash between sidecar writes",
    )?;

    // Legacy databases: verify (then discard) the old duplicate vectors file.
    check_legacy_vectors_file(path, meta)?;

    // Cross-check each mapped index against the loaded graph's vector store.
    // Internal indices are sparse and monotonic (never reused after an
    // upsert/delete) but every live index must resolve to a graph slot —
    // `ContiguousVectors` is dense in `0..count`, so `< count` is the exact
    // membership test. This external bound is strictly stronger than the
    // file's self-reported `next_idx`, which comes from the same untrusted
    // blob and cannot vouch for itself.
    validate_loaded_mappings(&mappings_data, graph_vector_count)?;

    Ok(super::sharded_mappings::ShardedMappings::from_parts(
        mappings_data.id_to_idx,
        mappings_data.idx_to_id,
        mappings_data.next_idx,
    ))
}

/// Rejects a sidecar whose generation does not match the meta generation,
/// which indicates a crash mid-save left the on-disk state inconsistent.
fn check_sidecar_generation(
    sidecar: &str,
    found: u64,
    expected: u64,
    crash_context: &str,
) -> std::io::Result<()> {
    if found != expected {
        return Err(invalid_data(format!(
            "incomplete save detected: {sidecar} generation {found} but meta generation \
             {expected} ({crash_context}, database state inconsistent)"
        )));
    }
    Ok(())
}

/// Builds an `InvalidData` I/O error for the load-time validation paths.
///
/// Single idiom shared by [`validate_loaded_vectors`] and
/// [`validate_loaded_mappings`] so the new validation code does not
/// re-spell `std::io::Error::new(InvalidData, …)` at every site.
fn invalid_data(msg: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

/// Validates vectors deserialized from an untrusted legacy
/// `native_vectors.bin`.
///
/// Every stored vector must have exactly `dimension` components and a unique
/// internal index. The payload is discarded after this check (PERF1), but a
/// malformed legacy file still proves on-disk corruption and must fail the
/// load exactly as it did when the file was authoritative.
///
/// # Errors
///
/// Returns `InvalidData` if any vector length differs from `dimension` or any
/// internal index is duplicated.
fn validate_loaded_vectors(vectors: &[(usize, Vec<f32>)], dimension: usize) -> std::io::Result<()> {
    let mut seen = std::collections::HashSet::with_capacity(vectors.len());
    for (idx, vec) in vectors {
        if vec.len() != dimension {
            return Err(invalid_data(format!(
                "vector at index {idx} has length {} but dimension is {dimension}",
                vec.len()
            )));
        }
        if !seen.insert(*idx) {
            return Err(invalid_data(format!(
                "duplicate internal index {idx} in persisted vectors"
            )));
        }
    }
    Ok(())
}

/// Validates id-mappings deserialized from an untrusted `native_mappings.bin`.
///
/// Enforces the bijection invariant `HnswIndex` relies on: every internal
/// index is `< max_exclusive` (the loaded graph's `ContiguousVectors` count,
/// dense in `0..count`), and `id_to_idx` / `idx_to_id` are exact inverses
/// of each other (same cardinality, every entry round-trips).
///
/// # Errors
///
/// Returns `InvalidData` on any out-of-range index or broken bijection.
fn validate_loaded_mappings(data: &HnswMappingsData, max_exclusive: usize) -> std::io::Result<()> {
    if data.id_to_idx.len() != data.idx_to_id.len() {
        return Err(invalid_data(format!(
            "mapping cardinality mismatch: id_to_idx={} idx_to_id={}",
            data.id_to_idx.len(),
            data.idx_to_id.len()
        )));
    }
    for (&id, &idx) in &data.id_to_idx {
        if idx >= max_exclusive {
            return Err(invalid_data(format!(
                "mapping id {id} resolves to index {idx} absent from the loaded \
                 vector store (graph has {max_exclusive} vector slots)"
            )));
        }
        if data.idx_to_id.get(&idx) != Some(&id) {
            return Err(invalid_data(format!(
                "mapping bijection broken for id {id} / idx {idx}"
            )));
        }
    }
    Ok(())
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
