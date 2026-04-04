//! Sparse index persistence: WAL, compaction, and mmap-based loading.
//!
//! All types and functions in this module are gated behind `#[cfg(feature = "persistence")]`.
//!
//! ## On-disk layout
//!
//! ```text
//! <collection_dir>/
//!   sparse.wal        # Write-ahead log (length-prefixed entries)
//!   sparse.idx        # Compacted posting lists (raw PostingEntry bytes)
//!   sparse.terms      # Term dictionary (postcard-serialized Vec<TermEntry>)
//!   sparse.meta       # Metadata (postcard-serialized SparseMeta)
//! ```

use std::io::{BufWriter, Write};
use std::path::Path;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use super::inverted_index::{FrozenSegment, SparseInvertedIndex};
use super::persistence_wal::{read_le_f32, read_le_u64};
use super::types::PostingEntry;
use crate::error::{Error, Result};

// Re-export WAL operations for backward compatibility.
pub use super::persistence_wal::{wal_append_delete, wal_append_upsert, wal_replay};

// WAL constants are in persistence_wal.rs

/// Number of replayed WAL entries that triggers automatic compaction on load.
const COMPACTION_REPLAY_THRESHOLD: u64 = 10_000;

// ---------------------------------------------------------------------------
// On-disk structures
// ---------------------------------------------------------------------------

/// Metadata header for the compacted sparse index.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct SparseMeta {
    pub(super) version: u32,
    pub(super) doc_count: u64,
    pub(super) term_count: u32,
}

/// Term dictionary entry mapping `term_id` to its posting range in `sparse.idx`.
#[derive(Debug, Serialize, Deserialize)]
struct TermEntry {
    term_id: u32,
    offset: u64,
    len: u32,
    max_weight: f32,
}

// ---------------------------------------------------------------------------
// WAL operations
// ---------------------------------------------------------------------------

// WAL operations (wal_append_upsert, wal_append_delete, wal_replay) are in persistence_wal.rs

/// Size of a single `PostingEntry` on disk (`u64` `doc_id` + `f32` weight, no padding).
const POSTING_DISK_SIZE: usize = 12; // 8 + 4, packed

const _: () = assert!(
    std::mem::size_of::<u64>() + std::mem::size_of::<f32>() == POSTING_DISK_SIZE,
    "POSTING_DISK_SIZE must match u64 + f32 packed size"
);

// ---------------------------------------------------------------------------
// Named sparse index helpers
// ---------------------------------------------------------------------------

/// Returns the file prefix for a named sparse index.
///
/// - Empty name `""` -> `"sparse"` (backward compat with unprefixed files)
/// - Named `"title"` -> `"sparse-title"`
fn sparse_file_prefix(name: &str) -> String {
    if name.is_empty() {
        "sparse".to_string()
    } else {
        format!("sparse-{name}")
    }
}

/// Compacts a named sparse index to disk using name-prefixed files.
///
/// Default name `""` uses unprefixed `sparse.*` files for backward compat.
///
/// # Errors
///
/// Returns an error if disk writes fail.
pub fn compact_named(dir: &Path, name: &str, index: &SparseInvertedIndex) -> Result<()> {
    let prefix = sparse_file_prefix(name);
    compact_with_prefix(dir, &prefix, index)
}

/// Loads a named sparse index from disk using name-prefixed files.
///
/// # Errors
///
/// Returns an error if files exist but are corrupt.
pub fn load_named_from_disk(dir: &Path, name: &str) -> Result<Option<SparseInvertedIndex>> {
    let prefix = sparse_file_prefix(name);
    load_from_disk_with_prefix(dir, &prefix)
}

/// Returns the WAL path for a named sparse index.
#[must_use]
pub fn wal_path_for_name(dir: &Path, name: &str) -> std::path::PathBuf {
    let prefix = sparse_file_prefix(name);
    dir.join(format!("{prefix}.wal"))
}

// ---------------------------------------------------------------------------
// Compaction
// ---------------------------------------------------------------------------

/// Compacts the in-memory index to disk using the default (unprefixed) file names.
///
/// Delegates to `compact_with_prefix` with prefix `"sparse"`.
///
/// # Errors
///
/// Returns an error if disk writes fail or if an internal index invariant is violated.
pub fn compact(dir: &Path, index: &SparseInvertedIndex) -> Result<()> {
    compact_with_prefix(dir, "sparse", index)
}

/// Compacts the in-memory index to disk using the given file prefix.
///
/// Files written: `{prefix}.idx`, `{prefix}.terms`, `{prefix}.meta`.
/// Truncates `{prefix}.wal` after successful compaction.
///
/// # Errors
///
/// Returns an error if disk writes fail or if the index's internal posting map is
/// inconsistent (a term ID present in the sorted key list is absent from the map).
fn compact_with_prefix(dir: &Path, prefix: &str, index: &SparseInvertedIndex) -> Result<()> {
    let merged = index.get_merged_postings_for_compaction();

    let mut term_ids: Vec<u32> = merged.keys().copied().collect();
    term_ids.sort_unstable();

    let term_entries = write_idx_tmp(dir, prefix, &term_ids, &merged)?;
    write_terms_tmp(dir, prefix, &term_entries)?;
    write_meta_tmp(dir, prefix, index.doc_count(), &term_ids)?;
    atomic_rename_compacted_files(dir, prefix)?;
    truncate_wal(dir, prefix)?;

    Ok(())
}

/// Writes the posting index file and returns the term dictionary entries.
fn write_idx_tmp(
    dir: &Path,
    prefix: &str,
    term_ids: &[u32],
    merged: &FxHashMap<u32, (Vec<PostingEntry>, f32)>,
) -> Result<Vec<TermEntry>> {
    let idx_tmp = dir.join(format!("{prefix}.idx.tmp"));
    let mut idx_file = BufWriter::new(
        std::fs::File::create(&idx_tmp)
            .map_err(|e| Error::SparseIndexError(format!("compact idx create: {e}")))?,
    );

    let mut term_entries: Vec<TermEntry> = Vec::with_capacity(term_ids.len());
    let mut current_offset: u64 = 0;

    for &term_id in term_ids {
        let (postings, max_weight) = lookup_term(term_id, merged)?;
        write_postings(&mut idx_file, postings)?;

        let byte_len = (postings.len() * POSTING_DISK_SIZE) as u64;
        term_entries.push(TermEntry {
            term_id,
            offset: current_offset,
            #[allow(clippy::cast_possible_truncation)]
            len: postings.len() as u32,
            max_weight: *max_weight,
        });
        current_offset += byte_len;
    }

    idx_file
        .flush()
        .map_err(|e| Error::SparseIndexError(format!("compact idx flush: {e}")))?;
    Ok(term_entries)
}

fn lookup_term(
    term_id: u32,
    merged: &FxHashMap<u32, (Vec<PostingEntry>, f32)>,
) -> Result<&(Vec<PostingEntry>, f32)> {
    merged.get(&term_id).ok_or_else(|| {
        Error::SparseIndexError(format!(
            "compact: term_id {term_id} absent from merged postings map"
        ))
    })
}

fn write_postings(w: &mut BufWriter<std::fs::File>, postings: &[PostingEntry]) -> Result<()> {
    for entry in postings {
        w.write_all(&entry.doc_id.to_le_bytes())
            .map_err(|e| Error::SparseIndexError(format!("compact idx write: {e}")))?;
        w.write_all(&entry.weight.to_le_bytes())
            .map_err(|e| Error::SparseIndexError(format!("compact idx write: {e}")))?;
    }
    Ok(())
}

/// Writes the term dictionary file.
fn write_terms_tmp(dir: &Path, prefix: &str, term_entries: &[TermEntry]) -> Result<()> {
    let terms_tmp = dir.join(format!("{prefix}.terms.tmp"));
    let terms_data = postcard::to_allocvec(term_entries)
        .map_err(|e| Error::SparseIndexError(format!("compact terms serialize: {e}")))?;
    std::fs::write(&terms_tmp, &terms_data)
        .map_err(|e| Error::SparseIndexError(format!("compact terms write: {e}")))
}

/// Writes the metadata file.
fn write_meta_tmp(dir: &Path, prefix: &str, doc_count: u64, term_ids: &[u32]) -> Result<()> {
    let meta_tmp = dir.join(format!("{prefix}.meta.tmp"));
    let meta = SparseMeta {
        version: 1,
        doc_count,
        #[allow(clippy::cast_possible_truncation)]
        term_count: term_ids.len() as u32,
    };
    let meta_data = postcard::to_allocvec(&meta)
        .map_err(|e| Error::SparseIndexError(format!("compact meta serialize: {e}")))?;
    std::fs::write(&meta_tmp, &meta_data)
        .map_err(|e| Error::SparseIndexError(format!("compact meta write: {e}")))
}

/// Atomically renames `.tmp` files to their final names.
fn atomic_rename_compacted_files(dir: &Path, prefix: &str) -> Result<()> {
    for ext in &["idx", "terms", "meta"] {
        std::fs::rename(
            dir.join(format!("{prefix}.{ext}.tmp")),
            dir.join(format!("{prefix}.{ext}")),
        )
        .map_err(|e| Error::SparseIndexError(format!("compact {ext} rename: {e}")))?;
    }
    Ok(())
}

/// Truncates the WAL file after successful compaction.
fn truncate_wal(dir: &Path, prefix: &str) -> Result<()> {
    let wal_path = dir.join(format!("{prefix}.wal"));
    if wal_path.exists() {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&wal_path)
            .map_err(|e| Error::SparseIndexError(format!("compact wal truncate: {e}")))?;
        file.set_len(0)
            .map_err(|e| Error::SparseIndexError(format!("compact wal truncate: {e}")))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Loading from disk
// ---------------------------------------------------------------------------

/// Loads a sparse index from disk using default (unprefixed) file names.
///
/// Delegates to `load_from_disk_with_prefix` with prefix `"sparse"`.
///
/// # Errors
///
/// Returns an error if files exist but are corrupt.
pub fn load_from_disk(dir: &Path) -> Result<Option<SparseInvertedIndex>> {
    load_from_disk_with_prefix(dir, "sparse")
}

/// Loads a sparse index from disk using the given file prefix.
///
/// Returns `Ok(None)` if no `{prefix}.meta` file is found (empty collection).
/// If `{prefix}.wal` exists, replays it after loading compacted data.
/// If replayed entries exceed the compaction threshold, triggers automatic compaction.
///
/// # Errors
///
/// Returns an error if files exist but cannot be read, deserialized, or contain
/// corrupt byte sequences that cannot be converted to the expected fixed-size arrays.
fn load_from_disk_with_prefix(dir: &Path, prefix: &str) -> Result<Option<SparseInvertedIndex>> {
    let meta_path = dir.join(format!("{prefix}.meta"));
    if !meta_path.exists() {
        return load_wal_only(dir, prefix);
    }

    let meta = load_and_validate_meta(&meta_path)?;
    let index = load_compacted_index(dir, prefix, &meta)?;

    let wal_path = dir.join(format!("{prefix}.wal"));
    let replayed = wal_replay(&wal_path, &index)?;
    if replayed >= COMPACTION_REPLAY_THRESHOLD {
        compact_with_prefix(dir, prefix, &index)?;
    }

    Ok(Some(index))
}

/// Handles the WAL-only scenario when no compacted files exist.
fn load_wal_only(dir: &Path, prefix: &str) -> Result<Option<SparseInvertedIndex>> {
    let wal_path = dir.join(format!("{prefix}.wal"));
    if !wal_path.exists() {
        return Ok(None);
    }
    let index = SparseInvertedIndex::new();
    let replayed = wal_replay(&wal_path, &index)?;
    if replayed == 0 {
        return Ok(None);
    }
    if replayed >= COMPACTION_REPLAY_THRESHOLD {
        compact_with_prefix(dir, prefix, &index)?;
    }
    Ok(Some(index))
}

/// Reads and validates the sparse metadata file.
fn load_and_validate_meta(meta_path: &Path) -> Result<SparseMeta> {
    let meta_data = std::fs::read(meta_path)
        .map_err(|e| Error::SparseIndexError(format!("load meta read: {e}")))?;
    let meta: SparseMeta = postcard::from_bytes(&meta_data)
        .map_err(|e| Error::SparseIndexError(format!("load meta deserialize: {e}")))?;
    if meta.version != 1 {
        return Err(Error::SparseIndexError(format!(
            "unsupported sparse meta version: {}",
            meta.version
        )));
    }
    Ok(meta)
}

/// Loads the compacted index from term dictionary and posting index files.
fn load_compacted_index(
    dir: &Path,
    prefix: &str,
    meta: &SparseMeta,
) -> Result<SparseInvertedIndex> {
    let terms_path = dir.join(format!("{prefix}.terms"));
    let terms_data = std::fs::read(&terms_path)
        .map_err(|e| Error::SparseIndexError(format!("load terms read: {e}")))?;
    let term_entries: Vec<TermEntry> = postcard::from_bytes(&terms_data)
        .map_err(|e| Error::SparseIndexError(format!("load terms deserialize: {e}")))?;

    let idx_path = dir.join(format!("{prefix}.idx"));
    let idx_data = std::fs::read(&idx_path)
        .map_err(|e| Error::SparseIndexError(format!("load idx read: {e}")))?;

    let postings = build_postings_from_idx(&idx_data, &term_entries)?;

    #[allow(clippy::cast_possible_truncation)]
    let frozen = FrozenSegment::new(postings, meta.doc_count as usize);
    Ok(SparseInvertedIndex::from_frozen_segment(frozen))
}

/// Deserializes the posting lists from a raw index buffer and its term dictionary.
///
/// Extracted to keep `load_from_disk` within the pedantic line-count budget.
fn build_postings_from_idx(
    idx_data: &[u8],
    term_entries: &[TermEntry],
) -> Result<FxHashMap<u32, (Vec<PostingEntry>, f32)>> {
    let mut postings: FxHashMap<u32, (Vec<PostingEntry>, f32)> = FxHashMap::default();

    for te in term_entries {
        #[allow(clippy::cast_possible_truncation)] // 32-bit target: file offsets won't exceed usize
        let start = te.offset as usize;
        let byte_count = (te.len as usize) * POSTING_DISK_SIZE;
        let end = start + byte_count;

        if end > idx_data.len() {
            return Err(Error::SparseIndexError(format!(
                "load idx: term {} offset {start}+{byte_count} exceeds file size {}",
                te.term_id,
                idx_data.len()
            )));
        }

        let mut entries = Vec::with_capacity(te.len as usize);
        let mut pos = start;
        for _ in 0..te.len {
            // We verified `end <= idx_data.len()` above, so every 12-byte window is in-bounds.
            // read_le_u64/f32 propagate rather than panic to catch future refactor regressions.
            let doc_id = read_le_u64(idx_data, pos, "load idx: corrupt doc_id bytes")?;
            pos += 8;
            let weight = read_le_f32(idx_data, pos, "load idx: corrupt weight bytes")?;
            pos += 4;
            entries.push(PostingEntry { doc_id, weight });
        }

        postings.insert(te.term_id, (entries, te.max_weight));
    }

    Ok(postings)
}
