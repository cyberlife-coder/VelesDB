//! Shared length-prefixed WAL framing helpers.
//!
//! The BM25 index WAL ([`crate::index::bm25_persistence_wal`]) and the
//! graph edge WAL ([`crate::collection::graph::edge_wal`]) use the same
//! on-disk discipline: append-mode `BufWriter`, fsync per flush, and a
//! `[u32 body_len]`-prefixed entry framing so unknown / truncated entries
//! are skippable during replay. Those file-open / write / flush / header
//! helpers live here so the two WALs do not duplicate them.
//!
//! The `context` argument is woven into error messages so each caller's
//! diagnostics stay distinguishable (e.g. "BM25 WAL open" vs
//! "Edge WAL open").

use std::io::{BufWriter, Write};
use std::path::Path;

use crate::error::{Error, Result};

/// Opens (creating if absent) the WAL file for append and wraps it in a
/// `BufWriter`.
///
/// # Errors
///
/// Returns [`Error::Index`] if the file cannot be opened.
pub(crate) fn open_wal_writer(wal_path: &Path, context: &str) -> Result<BufWriter<std::fs::File>> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(wal_path)
        .map_err(|e| Error::Index(format!("{context} open: {e}")))?;
    Ok(BufWriter::new(file))
}

/// Writes `bytes` to the WAL writer.
///
/// # Errors
///
/// Returns [`Error::Index`] if the write fails.
pub(crate) fn wal_write(
    w: &mut BufWriter<std::fs::File>,
    bytes: &[u8],
    context: &str,
) -> Result<()> {
    w.write_all(bytes)
        .map_err(|e| Error::Index(format!("{context} write: {e}")))
}

/// Flushes the `BufWriter` and fsyncs the underlying file.
///
/// # Errors
///
/// Returns [`Error::Index`] if the flush or fsync fails.
pub(crate) fn flush_wal(w: &mut BufWriter<std::fs::File>, context: &str) -> Result<()> {
    w.flush()
        .map_err(|e| Error::Index(format!("{context} flush: {e}")))?;
    w.get_ref()
        .sync_all()
        .map_err(|e| Error::Index(format!("{context} fsync: {e}")))
}

/// Truncates the WAL file to zero length. A missing file is a no-op.
///
/// # Errors
///
/// Returns [`Error::Index`] if the file exists but cannot be truncated.
pub(crate) fn wal_truncate(wal_path: &Path, context: &str) -> Result<()> {
    if !wal_path.exists() {
        return Ok(());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(wal_path)
        .map_err(|e| Error::Index(format!("{context} truncate open: {e}")))?;
    file.set_len(0)
        .map_err(|e| Error::Index(format!("{context} truncate: {e}")))
}

/// Reads the 4-byte little-endian length prefix at `pos`, returning
/// `(body_start, body_len)`. Returns `None` (logging at `warn`) when the
/// remaining bytes cannot hold a prefix — the torn-tail crash case.
pub(crate) fn read_entry_header(data: &[u8], pos: usize, context: &str) -> Option<(usize, usize)> {
    if pos + 4 > data.len() {
        tracing::warn!("{context} truncated at offset {pos}: not enough bytes for length prefix");
        return None;
    }
    let bytes: [u8; 4] = data[pos..pos + 4].try_into().ok()?;
    let body_len = u32::from_le_bytes(bytes) as usize;
    Some((pos + 4, body_len))
}
