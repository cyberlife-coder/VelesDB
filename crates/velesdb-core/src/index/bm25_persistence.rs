//! BM25 index persistence: atomic snapshot save/load.
//!
//! All types and functions in this module are gated behind
//! `#[cfg(feature = "persistence")]`.
//!
//! ## On-disk layout
//!
//! ```text
//! <collection_dir>/
//!   bm25.snapshot    # Postcard-serialized [`Bm25Snapshot`]
//!   bm25.wal         # Write-ahead log (see [`bm25_persistence_wal`])
//!   bm25.coverage    # A graph's coverage marker ([`Bm25Coverage`])
//! ```
//!
//! The snapshot captures the full in-memory state of the BM25 index
//! (documents, term frequencies, point/doc-id mappings, doc-count and
//! total length). The WAL captures mutations made after the most recent
//! snapshot. `load_snapshot` + `wal_replay` together restore the index
//! to its pre-shutdown state in O(snapshot) + O(WAL delta) time, which
//! replaces the prior O(N) payload-scan rebuild.
//!
//! ## Corruption handling
//!
//! `load_snapshot` returns `Ok(None)` only when the snapshot file is
//! absent (`NotFound`). Any other read error — including corrupt
//! bytes that fail postcard deserialization — surfaces as `Err`.
//! Silent fallback to the payload-rebuild path must be triggered by
//! the caller checking for `Ok(None)`; never by swallowing an `Err`.
//! See issue #618 for the Devin learning that motivates this
//! fail-fast contract.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::index::bm25::Bm25Index;
use crate::index::bm25_term_dict::parse_snapshot;
use crate::storage::atomic_write::atomic_write;
use crate::storage::snapshot::crc32_hash;

/// Snapshot filename under a collection directory.
pub(crate) const BM25_SNAPSHOT_FILENAME: &str = "bm25.snapshot";

/// Coverage marker filename under a graph collection's directory.
pub(crate) const BM25_COVERAGE_FILENAME: &str = "bm25.coverage";

/// Returns the absolute path to the BM25 snapshot file under `dir`.
#[must_use]
pub(crate) fn snapshot_path(dir: &Path) -> PathBuf {
    dir.join(BM25_SNAPSHOT_FILENAME)
}

/// Returns the absolute path to a graph's BM25 coverage marker under `dir`.
#[must_use]
pub(crate) fn coverage_path(dir: &Path) -> PathBuf {
    dir.join(BM25_COVERAGE_FILENAME)
}

/// The bytes of one snapshot file, as a marker binds to them: their length
/// and CRC32. Any other snapshot — rewritten by another writer, or not yet
/// written — has another fingerprint, save for a CRC collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SnapshotFingerprint {
    len: u64,
    crc32: u32,
}

impl SnapshotFingerprint {
    fn of(bytes: &[u8]) -> Self {
        Self {
            len: bytes.len() as u64,
            crc32: crc32_hash(bytes),
        }
    }
}

/// What a graph's BM25 snapshot covers, written beside it by the writer that
/// indexes every payload a graph stores — its nodes without an embedding
/// included, which an older writer left out. It vouches for the one
/// snapshot whose fingerprint it holds, and for every payload write before
/// `payloads_from` in the payload log — its BM25 change is in that snapshot
/// or in the WAL written after it, which `flush_full` guarantees by holding
/// `text_wal` exclusively from serializing the index through truncating the
/// WAL (`Collection::flush_bm25_index`); `Collection::open` indexes the
/// payloads written from there on again (`collection/core/bm25_coverage.rs`).
///
/// An older binary ignores the file and never writes it, so it cannot vouch
/// for what such a binary wrote: a snapshot it rewrites no longer matches,
/// and a node it stores lands in the payload log past `payloads_from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Bm25Coverage {
    /// The marker's own format, [`Self::FORMAT`]; any other is no marker.
    format: u32,
    /// The snapshot this marker vouches for.
    pub(crate) snapshot: SnapshotFingerprint,
    /// The payload log position up to which the snapshot and its WAL cover
    /// every payload write.
    pub(crate) payloads_from: u64,
}

impl Bm25Coverage {
    const FORMAT: u32 = 1;

    /// A marker vouching for `snapshot`, its payloads covered up to
    /// `payloads_from`.
    pub(crate) const fn new(snapshot: SnapshotFingerprint, payloads_from: u64) -> Self {
        Self {
            format: Self::FORMAT,
            snapshot,
            payloads_from,
        }
    }
}

/// Saves the BM25 index as an atomic snapshot under `dir/bm25.snapshot`,
/// returning the fingerprint of the bytes it wrote.
///
/// Uses the `write-tmp-fsync-rename` pattern to guarantee that a crash
/// mid-save never leaves a torn snapshot file observable by the next
/// startup.
///
/// # Errors
///
/// Returns [`Error::Index`] if serialization or disk I/O fails.
pub(crate) fn save_snapshot(dir: &Path, index: &Bm25Index) -> Result<SnapshotFingerprint> {
    let snapshot = index.to_snapshot();
    let bytes = postcard::to_allocvec(&snapshot)
        .map_err(|e| Error::Index(format!("BM25 snapshot serialize: {e}")))?;
    let final_path = snapshot_path(dir);
    atomic_write(&final_path, &bytes)
        .map_err(|e| Error::Index(format!("BM25 snapshot write: {e}")))?;
    Ok(SnapshotFingerprint::of(&bytes))
}

/// A snapshot read from disk: the index it holds, and the fingerprint of the
/// bytes it was read from.
pub(crate) struct StoredSnapshot {
    /// The index, migrated to the current format.
    pub(crate) index: Bm25Index,
    /// The fingerprint of the file's bytes.
    pub(crate) fingerprint: SnapshotFingerprint,
}

/// Loads the BM25 index from `dir/bm25.snapshot` if present, with the
/// fingerprint of the bytes it read.
///
/// - Returns `Ok(None)` when the snapshot file does not exist (backward
///   compat: the caller should fall back to the payload-rebuild path).
/// - Returns `Err(Error::Index(..))` when the file exists but cannot
///   be read or deserialized (corruption must surface loudly — never
///   silently fall back to rebuild, per issue #618 learning).
///
/// # Errors
///
/// Returns [`Error::Index`] when the file exists but is unreadable or
/// contains corrupt bytes that fail postcard deserialization.
pub(crate) fn load_snapshot(dir: &Path) -> Result<Option<StoredSnapshot>> {
    let path = snapshot_path(dir);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Index(format!("BM25 snapshot read: {e}"))),
    };
    // `parse_snapshot` dispatches on the leading version field: current
    // snapshots deserialize directly, version-1 (pre-#2090) snapshots are
    // migrated in memory. Corruption surfaces as `Err`, per the contract
    // above.
    let snapshot = parse_snapshot(&bytes)?;
    Ok(Some(StoredSnapshot {
        index: Bm25Index::from_snapshot(snapshot)?,
        fingerprint: SnapshotFingerprint::of(&bytes),
    }))
}

/// Writes a graph's coverage marker under `dir/bm25.coverage`, atomically
/// and durably — after the snapshot it vouches for is durable.
///
/// # Errors
///
/// Returns [`Error::Index`] if serialization or disk I/O fails.
pub(crate) fn save_coverage(dir: &Path, coverage: &Bm25Coverage) -> Result<()> {
    let bytes = postcard::to_allocvec(coverage)
        .map_err(|e| Error::Index(format!("BM25 coverage serialize: {e}")))?;
    atomic_write(&coverage_path(dir), &bytes)
        .map_err(|e| Error::Index(format!("BM25 coverage write: {e}")))
}

/// Reads a graph's coverage marker. A marker that is absent, unreadable, of
/// another format or undecodable vouches for nothing: `None`, and the caller
/// rebuilds the index from the payloads it is derived from.
#[must_use]
pub(crate) fn load_coverage(dir: &Path) -> Option<Bm25Coverage> {
    let bytes = std::fs::read(coverage_path(dir)).ok()?;
    postcard::from_bytes::<Bm25Coverage>(&bytes)
        .ok()
        .filter(|coverage| coverage.format == Bm25Coverage::FORMAT)
}

// Atomic snapshot writes use the shared `crate::storage::atomic_write` helper
// (write-tmp + fsync + rename), so the crash-safety logic lives in one place.
