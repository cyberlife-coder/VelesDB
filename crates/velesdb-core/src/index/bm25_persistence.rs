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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};
use crate::index::bm25::{Bm25Index, Bm25Snapshot};

/// Snapshot filename under a collection directory.
pub(crate) const BM25_SNAPSHOT_FILENAME: &str = "bm25.snapshot";

/// Returns the absolute path to the BM25 snapshot file under `dir`.
#[must_use]
pub(crate) fn snapshot_path(dir: &Path) -> PathBuf {
    dir.join(BM25_SNAPSHOT_FILENAME)
}

/// Saves the BM25 index as an atomic snapshot under `dir/bm25.snapshot`.
///
/// Uses the `write-tmp-fsync-rename` pattern to guarantee that a crash
/// mid-save never leaves a torn snapshot file observable by the next
/// startup.
///
/// # Errors
///
/// Returns [`Error::Index`] if serialization or disk I/O fails.
pub(crate) fn save_snapshot(dir: &Path, index: &Bm25Index) -> Result<()> {
    let snapshot = index.to_snapshot();
    let bytes = postcard::to_allocvec(&snapshot)
        .map_err(|e| Error::Index(format!("BM25 snapshot serialize: {e}")))?;
    let final_path = snapshot_path(dir);
    atomic_write(&final_path, &bytes).map_err(|e| Error::Index(format!("BM25 snapshot write: {e}")))
}

/// Loads the BM25 index from `dir/bm25.snapshot` if present.
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
pub(crate) fn load_snapshot(dir: &Path) -> Result<Option<Bm25Index>> {
    let path = snapshot_path(dir);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Index(format!("BM25 snapshot read: {e}"))),
    };
    let snapshot: Bm25Snapshot = postcard::from_bytes(&bytes)
        .map_err(|e| Error::Index(format!("BM25 snapshot deserialize: {e}")))?;
    Ok(Some(Bm25Index::from_snapshot(snapshot)))
}

// ---------------------------------------------------------------------------
// Atomic write (write-tmp-fsync-rename)
// ---------------------------------------------------------------------------

/// Process-wide counter to produce unique tmp suffixes even when two
/// threads race to save the same snapshot.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `data` to `final_path` atomically: stage to a uniquely-named
/// tmp file in the same directory, flush + fsync, then rename over.
///
/// Mirrors the pattern in `index::hnsw::persistence::atomic_write`. The
/// tmp suffix combines PID, thread ID and a monotonically-increasing
/// counter to avoid collisions under concurrent saves.
fn atomic_write(final_path: &Path, data: &[u8]) -> std::io::Result<()> {
    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let tid = std::thread::current().id();

    let file_name = final_path.file_name().unwrap_or_default().to_string_lossy();
    let tmp_name = format!("{file_name}.tmp.{pid}.{tid:?}.{seq}");
    let tmp_path = final_path.with_file_name(&tmp_name);

    let result = atomic_write_inner(&tmp_path, final_path, data);
    if result.is_err() {
        // Best-effort cleanup — ignore any follow-up error.
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn atomic_write_inner(tmp_path: &Path, final_path: &Path, data: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(tmp_path)?;
    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(data)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    std::fs::rename(tmp_path, final_path)
}
