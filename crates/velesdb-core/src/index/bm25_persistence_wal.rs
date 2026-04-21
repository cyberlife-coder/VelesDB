//! BM25 index WAL: append + replay for incremental persistence.
//!
//! The WAL captures `add_document` / `remove_document` mutations applied
//! after the most recent snapshot. On collection open, the snapshot is
//! loaded first and the WAL is replayed on top to bring the index
//! up-to-date. After a successful `save_snapshot`, the WAL must be
//! truncated via [`wal_truncate`] so the next open replays zero entries.
//!
//! ## On-disk entry layout (length-prefixed, little-endian)
//!
//! ```text
//! Add:    [u32 body_len][u8 0x01][u64 point_id][u32 text_len][text bytes]
//! Remove: [u32 body_len][u8 0x02][u64 point_id]
//! ```
//!
//! `body_len` is the byte count *after* the prefix — it lets the replay
//! loop skip unknown / corrupt entries without aborting the whole
//! recovery. A truncated final entry (common on crash) is logged at
//! `warn` level and skipped rather than surfacing as an error.
//!
//! ## Crash-safety ordering
//!
//! Callers MUST invoke `wal_append_*` BEFORE applying the corresponding
//! in-memory mutation. If the process crashes between the two, replay
//! reconstructs the mutation on next open. The WAL is fsynced on every
//! append so that a power-cut after append but before any subsequent
//! write still replays the entry correctly.
//!
//! All functions here are gated behind `#[cfg(feature = "persistence")]`.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::index::bm25::Bm25Index;

const WAL_OP_ADD: u8 = 0x01;
const WAL_OP_REMOVE: u8 = 0x02;

/// WAL filename under a collection directory.
const BM25_WAL_FILENAME: &str = "bm25.wal";

/// Header sizes used to validate truncated entries during replay.
const ADD_ENTRY_HEADER: usize = 1 + 8 + 4; // op + point_id + text_len
const REMOVE_ENTRY_HEADER: usize = 1 + 8; // op + point_id

/// Returns the absolute path to the BM25 WAL file under `dir`.
#[must_use]
pub(crate) fn wal_path_for_bm25(dir: &Path) -> PathBuf {
    dir.join(BM25_WAL_FILENAME)
}

// ---------------------------------------------------------------------------
// Append operations (hot path — keep `#[inline]`)
// ---------------------------------------------------------------------------

/// Appends an `add_document(id, text)` mutation to the BM25 WAL.
///
/// Callers MUST invoke this BEFORE applying the mutation in-memory so
/// that a crash between WAL append and in-memory apply replays the
/// mutation on next open (WAL-before-apply crash-safety ordering).
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file cannot be opened or
/// written, or if the text is too large to encode in a `u32`-prefixed
/// entry.
#[inline]
pub(crate) fn wal_append_add_document(wal_path: &Path, id: u64, text: &str) -> Result<()> {
    let text_bytes = text.as_bytes();
    let text_len = u32::try_from(text_bytes.len()).map_err(|_| {
        Error::Index(format!(
            "BM25 WAL: text too large ({} bytes) to encode",
            text_bytes.len()
        ))
    })?;

    // body = op(1) + point_id(8) + text_len(4) + text_bytes
    let body_len = u32::try_from(ADD_ENTRY_HEADER)
        .ok()
        .and_then(|h| h.checked_add(text_len))
        .ok_or_else(|| {
            Error::Index(format!(
                "BM25 WAL: entry too large (text_len={text_len}) to fit in u32 prefix"
            ))
        })?;

    let mut w = open_wal_writer(wal_path)?;
    wal_write(&mut w, &body_len.to_le_bytes())?;
    wal_write(&mut w, &[WAL_OP_ADD])?;
    wal_write(&mut w, &id.to_le_bytes())?;
    wal_write(&mut w, &text_len.to_le_bytes())?;
    wal_write(&mut w, text_bytes)?;
    flush_wal(&mut w)
}

/// Appends a `remove_document(id)` mutation to the BM25 WAL.
///
/// Callers MUST invoke this BEFORE applying the mutation in-memory
/// (WAL-before-apply crash-safety ordering).
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file cannot be opened or
/// written.
#[inline]
pub(crate) fn wal_append_remove_document(wal_path: &Path, id: u64) -> Result<()> {
    // Fits in a `u32` by construction — constant header size = 9.
    let body_len = u32::try_from(REMOVE_ENTRY_HEADER).expect("REMOVE_ENTRY_HEADER <= u32::MAX");
    let mut w = open_wal_writer(wal_path)?;
    wal_write(&mut w, &body_len.to_le_bytes())?;
    wal_write(&mut w, &[WAL_OP_REMOVE])?;
    wal_write(&mut w, &id.to_le_bytes())?;
    flush_wal(&mut w)
}

/// Truncates the BM25 WAL file to zero length.
///
/// Called after a successful `save_snapshot` to guarantee that the next
/// open replays zero WAL entries. A missing WAL file is a no-op.
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file exists but cannot be
/// truncated.
pub(crate) fn wal_truncate(wal_path: &Path) -> Result<()> {
    if !wal_path.exists() {
        return Ok(());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(wal_path)
        .map_err(|e| Error::Index(format!("BM25 WAL truncate open: {e}")))?;
    file.set_len(0)
        .map_err(|e| Error::Index(format!("BM25 WAL truncate: {e}")))
}

// ---------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------

/// Replays the BM25 WAL against the provided `index`, returning the
/// number of entries applied.
///
/// Missing WAL file returns `Ok(0)`. A truncated final entry (partial
/// crash during append) is logged at `warn` level and skipped. Unknown
/// opcodes are logged and skipped without aborting replay.
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file exists but cannot be read,
/// or if a complete entry contains corrupt byte sequences that cannot
/// be decoded.
pub(crate) fn wal_replay(wal_path: &Path, index: &Bm25Index) -> Result<u64> {
    let data = match std::fs::read(wal_path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(Error::Index(format!("BM25 WAL read: {e}"))),
    };

    let mut pos = 0usize;
    let mut count = 0u64;

    while pos < data.len() {
        let Some((body_start, body_len)) = read_entry_header(&data, pos) else {
            break;
        };
        pos = body_start;
        if pos + body_len > data.len() {
            tracing::warn!(
                "BM25 WAL truncated at offset {body_start}: declared {body_len} bytes but only {} remain",
                data.len() - pos
            );
            break;
        }
        let op = data[pos];
        pos += 1;
        let applied = replay_single_entry(&data, op, &mut pos, body_start, body_len, index)?;
        count += applied;
        // Defensive: align `pos` to the entry boundary even when a
        // replayer short-circuited (e.g. skipped a malformed entry).
        if pos < body_start + body_len {
            pos = body_start + body_len;
        }
    }

    Ok(count)
}

/// Reads the `u32` length prefix, returning `(body_start, body_len)`.
fn read_entry_header(data: &[u8], pos: usize) -> Option<(usize, usize)> {
    if pos + 4 > data.len() {
        tracing::warn!("BM25 WAL truncated at offset {pos}: not enough bytes for length prefix");
        return None;
    }
    let bytes: [u8; 4] = data[pos..pos + 4].try_into().ok()?;
    let body_len = u32::from_le_bytes(bytes) as usize;
    Some((pos + 4, body_len))
}

/// Dispatches a single WAL entry to the appropriate in-memory mutation.
fn replay_single_entry(
    data: &[u8],
    op: u8,
    pos: &mut usize,
    body_start: usize,
    body_len: usize,
    index: &Bm25Index,
) -> Result<u64> {
    match op {
        WAL_OP_ADD => replay_add_entry(data, pos, body_start, body_len, index),
        WAL_OP_REMOVE => replay_remove_entry(data, pos, index),
        unknown => {
            tracing::warn!("BM25 WAL unknown op 0x{unknown:02x} at offset {body_start}");
            *pos = body_start + body_len;
            Ok(0)
        }
    }
}

/// Replays a single `add_document` entry.
fn replay_add_entry(
    data: &[u8],
    pos: &mut usize,
    body_start: usize,
    body_len: usize,
    index: &Bm25Index,
) -> Result<u64> {
    if body_len < ADD_ENTRY_HEADER {
        tracing::warn!("BM25 WAL add entry too short at offset {body_start}");
        *pos = body_start + body_len;
        return Ok(0);
    }
    let id = read_le_u64(data, *pos)?;
    *pos += 8;
    let text_len = read_le_u32(data, *pos)? as usize;
    *pos += 4;
    let text_end = *pos + text_len;
    if text_end > body_start + body_len || text_end > data.len() {
        tracing::warn!("BM25 WAL add entry truncated at offset {body_start}");
        *pos = body_start + body_len;
        return Ok(0);
    }
    let text = std::str::from_utf8(&data[*pos..text_end])
        .map_err(|e| Error::Index(format!("BM25 WAL add: invalid utf8 at {body_start}: {e}")))?;
    index.add_document(id, text);
    *pos = text_end;
    Ok(1)
}

/// Replays a single `remove_document` entry.
fn replay_remove_entry(data: &[u8], pos: &mut usize, index: &Bm25Index) -> Result<u64> {
    let id = read_le_u64(data, *pos)?;
    *pos += 8;
    index.remove_document(id);
    Ok(1)
}

#[inline]
fn read_le_u64(data: &[u8], pos: usize) -> Result<u64> {
    data[pos..pos + 8]
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| Error::Index(format!("BM25 WAL: corrupt u64 at offset {pos}")))
}

#[inline]
fn read_le_u32(data: &[u8], pos: usize) -> Result<u32> {
    data[pos..pos + 4]
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| Error::Index(format!("BM25 WAL: corrupt u32 at offset {pos}")))
}

// ---------------------------------------------------------------------------
// File-open helpers
// ---------------------------------------------------------------------------

fn open_wal_writer(wal_path: &Path) -> Result<BufWriter<std::fs::File>> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(wal_path)
        .map_err(|e| Error::Index(format!("BM25 WAL open: {e}")))?;
    Ok(BufWriter::new(file))
}

fn wal_write(w: &mut BufWriter<std::fs::File>, bytes: &[u8]) -> Result<()> {
    w.write_all(bytes)
        .map_err(|e| Error::Index(format!("BM25 WAL write: {e}")))
}

fn flush_wal(w: &mut BufWriter<std::fs::File>) -> Result<()> {
    w.flush()
        .map_err(|e| Error::Index(format!("BM25 WAL flush: {e}")))?;
    w.get_ref()
        .sync_all()
        .map_err(|e| Error::Index(format!("BM25 WAL fsync: {e}")))
}
