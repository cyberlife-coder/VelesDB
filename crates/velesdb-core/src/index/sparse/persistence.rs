//! Sparse index persistence: WAL, compaction, and mmap-based loading.
//!
//! All types and functions in this module are gated behind `#[cfg(feature = "persistence")]`.
//!
//! ## On-disk layout
//!
//! ```text
//! <collection_dir>/
//!   sparse.snapshot   # Durable manifest selecting one complete slot
//!   sparse.wal        # Generation-tagged write-ahead log
//!   sparse.{idx,terms,meta}        # Snapshot slot 0 / legacy layout
//!   .sparse.next.{idx,terms,meta}  # Hidden snapshot slot 1
//! ```
//!
//! Databases without `sparse.snapshot` keep loading directly from the legacy
//! slot-0 files, so the format upgrade requires no migration.

use std::io::{BufWriter, Write};
use std::path::Path;

#[cfg(test)]
use std::cell::Cell;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use super::inverted_index::{FrozenSegment, SparseInvertedIndex};
use super::persistence_generation::{
    active_snapshot, prepare_snapshot, publish_snapshot, SnapshotPaths,
};
use super::persistence_wal::{
    read_le_f32, read_le_u64, reset_wal, sync_wal, wal_replay_for_generation,
};
use super::types::PostingEntry;
use crate::error::{Error, Result};
use crate::storage::atomic_write::{atomic_write, atomic_write_with};

// Re-export WAL operations for backward compatibility.
pub use super::persistence_wal::{
    wal_append_delete, wal_append_delete_batch, wal_append_upsert, wal_append_upsert_batch,
    wal_replay,
};

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
/// Writes the inactive snapshot slot, commits it through `{prefix}.snapshot`,
/// then durably resets `{prefix}.wal` to the committed generation.
///
/// # Errors
///
/// Returns an error if disk writes fail or if the index's internal posting map is
/// inconsistent (a term ID present in the sorted key list is absent from the map).
fn compact_with_prefix(dir: &Path, prefix: &str, index: &SparseInvertedIndex) -> Result<()> {
    sync_wal(&dir.join(format!("{prefix}.wal")))?;
    let merged = index.get_merged_postings_for_compaction();
    let mut term_ids: Vec<u32> = merged.keys().copied().collect();
    term_ids.sort_unstable();

    let pending = prepare_snapshot(dir, prefix)?;
    check_publication_boundary(PublicationBoundary::IndexPromotion)?;
    let term_entries = write_idx_snapshot(&pending.paths.idx, &term_ids, &merged)?;
    check_publication_boundary(PublicationBoundary::TermsPromotion)?;
    write_terms_snapshot(&pending.paths.terms, &term_entries)?;
    check_publication_boundary(PublicationBoundary::MetaPromotion)?;
    write_meta_snapshot(&pending.paths.meta, index.doc_count(), &term_ids)?;
    check_publication_boundary(PublicationBoundary::CommitPoint)?;
    publish_snapshot(dir, prefix, &pending)?;
    check_publication_boundary(PublicationBoundary::WalTruncation)?;
    reset_wal(&dir.join(format!("{prefix}.wal")), pending.generation())?;
    Ok(())
}

/// Writes the posting index file and returns the term dictionary entries.
fn write_idx_snapshot(
    path: &Path,
    term_ids: &[u32],
    merged: &FxHashMap<u32, (Vec<PostingEntry>, f32)>,
) -> Result<Vec<TermEntry>> {
    atomic_write_with(path, |writer| write_idx_contents(writer, term_ids, merged))
        .map_err(|e| Error::SparseIndexError(format!("compact idx publish: {e}")))
}

fn write_idx_contents(
    writer: &mut BufWriter<std::fs::File>,
    term_ids: &[u32],
    merged: &FxHashMap<u32, (Vec<PostingEntry>, f32)>,
) -> Result<Vec<TermEntry>> {
    let mut entries = Vec::with_capacity(term_ids.len());
    let mut offset = 0_u64;
    for &term_id in term_ids {
        let (postings, max_weight) = lookup_term(term_id, merged)?;
        write_postings(writer, postings)?;
        entries.push(term_entry(term_id, postings, *max_weight, offset));
        offset += (postings.len() * POSTING_DISK_SIZE) as u64;
    }
    Ok(entries)
}

fn term_entry(term_id: u32, postings: &[PostingEntry], max_weight: f32, offset: u64) -> TermEntry {
    TermEntry {
        term_id,
        offset,
        #[allow(clippy::cast_possible_truncation)]
        len: postings.len() as u32,
        max_weight,
    }
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
fn write_terms_snapshot(path: &Path, term_entries: &[TermEntry]) -> Result<()> {
    let terms_data = postcard::to_allocvec(term_entries)
        .map_err(|e| Error::SparseIndexError(format!("compact terms serialize: {e}")))?;
    atomic_write(path, &terms_data)
        .map_err(|e| Error::SparseIndexError(format!("compact terms publish: {e}")))
}

/// Writes the metadata file.
fn write_meta_snapshot(path: &Path, doc_count: u64, term_ids: &[u32]) -> Result<()> {
    let meta = SparseMeta {
        version: 1,
        doc_count,
        #[allow(clippy::cast_possible_truncation)]
        term_count: term_ids.len() as u32,
    };
    let meta_data = postcard::to_allocvec(&meta)
        .map_err(|e| Error::SparseIndexError(format!("compact meta serialize: {e}")))?;
    atomic_write(path, &meta_data)
        .map_err(|e| Error::SparseIndexError(format!("compact meta publish: {e}")))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PublicationBoundary {
    IndexPromotion,
    TermsPromotion,
    MetaPromotion,
    CommitPoint,
    WalTruncation,
}

#[cfg(not(test))]
#[allow(clippy::unnecessary_wraps)] // Test builds return injected boundary failures.
fn check_publication_boundary(_boundary: PublicationBoundary) -> Result<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static PUBLICATION_FAULT: Cell<Option<PublicationBoundary>> = const { Cell::new(None) };
}

#[cfg(test)]
fn check_publication_boundary(boundary: PublicationBoundary) -> Result<()> {
    PUBLICATION_FAULT.with(|fault| {
        if fault.get() == Some(boundary) {
            fault.set(None);
            return Err(Error::SparseIndexError(format!(
                "fault injected at {boundary:?}"
            )));
        }
        Ok(())
    })
}

#[cfg(test)]
pub(super) struct PublicationFaultGuard(Option<PublicationBoundary>);

#[cfg(test)]
impl PublicationFaultGuard {
    pub(super) fn inject(boundary: PublicationBoundary) -> Self {
        let previous = PUBLICATION_FAULT.with(|fault| fault.replace(Some(boundary)));
        Self(previous)
    }
}

#[cfg(test)]
impl Drop for PublicationFaultGuard {
    fn drop(&mut self) {
        PUBLICATION_FAULT.with(|fault| fault.set(self.0));
    }
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
    let Some(snapshot) = active_snapshot(dir, prefix)? else {
        return load_wal_only(dir, prefix);
    };

    let meta = load_and_validate_meta(&snapshot.paths.meta)?;
    let index = load_compacted_index(&snapshot.paths, &meta)?;

    let wal_path = dir.join(format!("{prefix}.wal"));
    let replayed = wal_replay_for_generation(&wal_path, &index, snapshot.wal_generation)?;
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
fn load_compacted_index(paths: &SnapshotPaths, meta: &SparseMeta) -> Result<SparseInvertedIndex> {
    let terms_data = std::fs::read(&paths.terms)
        .map_err(|e| Error::SparseIndexError(format!("load terms read: {e}")))?;
    let term_entries: Vec<TermEntry> = postcard::from_bytes(&terms_data)
        .map_err(|e| Error::SparseIndexError(format!("load terms deserialize: {e}")))?;

    // Untrusted input: the decoded term dictionary length must match the count
    // recorded in the metadata header. A mismatch means a corrupt or crafted
    // file (e.g. a `term_count` that disagrees with the actual postings layout).
    if term_entries.len() != meta.term_count as usize {
        return Err(Error::SparseIndexError(format!(
            "load terms: term count mismatch (meta {} != decoded {})",
            meta.term_count,
            term_entries.len()
        )));
    }

    let idx_data = std::fs::read(&paths.idx)
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
        let entries = read_term_postings(idx_data, te)?;
        postings.insert(te.term_id, (entries, te.max_weight));
    }

    Ok(postings)
}

/// Validates an (untrusted) term's byte range against the file length and
/// returns its start offset. All arithmetic is done in `u64` so a crafted
/// `offset`/`len` cannot truncate on a 32-bit target and slip past the check.
fn validate_term_range(idx_data_len: usize, te: &TermEntry) -> Result<usize> {
    let byte_count = u64::from(te.len)
        .checked_mul(POSTING_DISK_SIZE as u64)
        .ok_or_else(|| {
            Error::SparseIndexError(format!("load idx: term {} len overflow", te.term_id))
        })?;
    let end = te.offset.checked_add(byte_count).ok_or_else(|| {
        Error::SparseIndexError(format!("load idx: term {} range overflow", te.term_id))
    })?;

    if end > idx_data_len as u64 {
        return Err(Error::SparseIndexError(format!(
            "load idx: term {} offset {}+{byte_count} exceeds file size {idx_data_len}",
            te.term_id, te.offset,
        )));
    }
    // Safe to narrow now: `end <= idx_data_len <= usize::MAX`, so `offset` fits.
    usize::try_from(te.offset).map_err(|_| {
        Error::SparseIndexError(format!("load idx: term {} offset too large", te.term_id))
    })
}

/// Reads one term's posting list (`te.len` 12-byte `doc_id`/weight pairs) after
/// validating its byte range fits within `idx_data`.
fn read_term_postings(idx_data: &[u8], te: &TermEntry) -> Result<Vec<PostingEntry>> {
    let start = validate_term_range(idx_data.len(), te)?;
    let mut entries = Vec::with_capacity(te.len as usize);
    let mut pos = start;
    for _ in 0..te.len {
        // The range was verified above, so every 12-byte window is in-bounds.
        // read_le_u64/f32 propagate rather than panic to catch future regressions.
        let doc_id = read_le_u64(idx_data, pos, "load idx: corrupt doc_id bytes")?;
        pos += 8;
        let weight = read_le_f32(idx_data, pos, "load idx: corrupt weight bytes")?;
        pos += 4;
        entries.push(PostingEntry { doc_id, weight });
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "offset_bound_tests.rs"]
mod offset_bound_tests;
