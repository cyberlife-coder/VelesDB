// TODO(US-389): remove these allows after commit 3 implements the WAL
// bodies (bringing them out of stub state) and commit 4 wires the
// lifecycle integration. Until then the stubs are infallible, which
// confuses `clippy::unnecessary_wraps` and `must_use_candidate`.
#![allow(dead_code)]
#![allow(clippy::unnecessary_wraps)]

//! BM25 index WAL: append + replay for incremental persistence.
//!
//! The WAL captures `add_document` / `remove_document` mutations applied
//! after the most recent snapshot. On collection open, the snapshot is
//! loaded first and the WAL is replayed on top to bring the index
//! up-to-date. After a successful `save_snapshot`, the WAL must be
//! truncated via [`wal_truncate`] so the next open replays zero entries.
//!
//! All functions here are gated behind `#[cfg(feature = "persistence")]`.
//!
//! ## Implementation status
//!
//! Commit 1 (this file) ships only signatures — the WAL body is
//! implemented in commit 3. The persistence tests therefore fail until
//! commit 3 lands, which is the intended TDD red/green transition.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::index::bm25::Bm25Index;

/// WAL filename under a collection directory.
const BM25_WAL_FILENAME: &str = "bm25.wal";

/// Returns the absolute path to the BM25 WAL file under `dir`.
#[must_use]
pub(crate) fn wal_path_for_bm25(dir: &Path) -> PathBuf {
    dir.join(BM25_WAL_FILENAME)
}

/// Appends an `add_document(id, text)` mutation to the BM25 WAL.
///
/// STUB — commit 3 implements the body. Currently a no-op so the
/// TDD test suite fails the replay assertions (red anchor).
///
/// # Errors
///
/// Infallible in the stub; commit 3 will return I/O errors.
#[inline]
pub(crate) fn wal_append_add_document(wal_path: &Path, id: u64, text: &str) -> Result<()> {
    let _ = (wal_path, id, text);
    Ok(())
}

/// Appends a `remove_document(id)` mutation to the BM25 WAL.
///
/// STUB — commit 3 implements the body.
///
/// # Errors
///
/// Infallible in the stub; commit 3 will return I/O errors.
#[inline]
pub(crate) fn wal_append_remove_document(wal_path: &Path, id: u64) -> Result<()> {
    let _ = (wal_path, id);
    Ok(())
}

/// Truncates the BM25 WAL file to zero length.
///
/// STUB — commit 3 implements the body.
///
/// # Errors
///
/// Infallible in the stub.
pub(crate) fn wal_truncate(wal_path: &Path) -> Result<()> {
    let _ = wal_path;
    Ok(())
}

/// Replays the BM25 WAL against the provided `index`, returning the
/// number of entries applied.
///
/// STUB — commit 3 implements the body. Returns 0 so the TDD tests
/// fail the replay count assertions (red anchor).
///
/// # Errors
///
/// Infallible in the stub.
pub(crate) fn wal_replay(wal_path: &Path, index: &Bm25Index) -> Result<u64> {
    let _ = (wal_path, index);
    Ok(0)
}
