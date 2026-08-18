//! Sparse index WAL replay logic.
//!
//! Extracted from `persistence.rs` to reduce NLOC below the 500 threshold.

use super::inverted_index::SparseInvertedIndex;
use super::persistence_generation::committed_generation_for_wal;
use super::types::SparseVector;
use crate::error::{Error, Result};
use crate::storage::atomic_write::{atomic_write, sync_parent_directory};

use std::io::{BufWriter, Read, Write};
use std::path::Path;

const WAL_OP_UPSERT: u8 = 0x01;
const WAL_OP_DELETE: u8 = 0x02;
const WAL_HEADER_MAGIC: [u8; 4] = *b"VSWL";
const WAL_HEADER_VERSION: u8 = 1;
const WAL_HEADER_LEN: usize = 13;

// ---------------------------------------------------------------------------
// Byte-parsing helpers
// ---------------------------------------------------------------------------

/// Reads a little-endian `u64` from `data[pos..pos+8]`.
#[inline]
pub(super) fn read_le_u64(data: &[u8], pos: usize, context: &str) -> Result<u64> {
    data[pos..pos + 8]
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| Error::SparseIndexError(format!("{context} at offset {pos}")))
}

/// Reads a little-endian `u32` from `data[pos..pos+4]`.
#[inline]
pub(super) fn read_le_u32(data: &[u8], pos: usize, context: &str) -> Result<u32> {
    data[pos..pos + 4]
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| Error::SparseIndexError(format!("{context} at offset {pos}")))
}

/// Reads a little-endian `f32` from `data[pos..pos+4]`.
#[inline]
pub(super) fn read_le_f32(data: &[u8], pos: usize, context: &str) -> Result<f32> {
    data[pos..pos + 4]
        .try_into()
        .map(f32::from_le_bytes)
        .map_err(|_| Error::SparseIndexError(format!("{context} at offset {pos}")))
}

// ---------------------------------------------------------------------------
// WAL write operations
// ---------------------------------------------------------------------------

/// Appends an upsert entry to the sparse WAL.
///
/// # Errors
///
/// Returns an error if the WAL file cannot be opened or written.
pub fn wal_append_upsert(wal_path: &Path, point_id: u64, vector: &SparseVector) -> Result<()> {
    #[allow(clippy::cast_possible_truncation)] // nnz bounded by sparse vector dimension count
    let nnz = vector.nnz() as u32;
    let total_len = compute_upsert_entry_len(nnz)?;

    let mut w = open_wal_writer(wal_path)?;
    write_upsert_header(&mut w, total_len, point_id, nnz)?;
    write_term_value_pairs(&mut w, &vector.indices, &vector.values)?;
    flush_wal(&mut w, wal_path)
}

/// Writes the upsert WAL entry header (length prefix, opcode, point ID, nnz).
fn write_upsert_header(
    w: &mut BufWriter<std::fs::File>,
    total_len: u32,
    point_id: u64,
    nnz: u32,
) -> Result<()> {
    wal_write(w, &total_len.to_le_bytes())?;
    wal_write(w, &[WAL_OP_UPSERT])?;
    wal_write(w, &point_id.to_le_bytes())?;
    wal_write(w, &nnz.to_le_bytes())
}

/// Writes sparse vector term-value pairs to the WAL.
fn write_term_value_pairs(
    w: &mut BufWriter<std::fs::File>,
    indices: &[u32],
    values: &[f32],
) -> Result<()> {
    for (&idx, &val) in indices.iter().zip(values.iter()) {
        wal_write(w, &idx.to_le_bytes())?;
        wal_write(w, &val.to_le_bytes())?;
    }
    Ok(())
}

/// Flushes AND fsyncs the WAL writer, mapping I/O errors to `SparseIndexError`.
///
/// A sparse upsert/delete is acknowledged only after this returns `Ok`, and the
/// sparse vectors live nowhere else — a lost WAL entry is not rebuilt from any
/// snapshot or payload. So the acked append path must be durable against power
/// loss, exactly like the BM25 and edge WALs: this delegates to the shared
/// `wal_framing::flush_wal` (a single `flush` + `sync_all`) rather than a
/// buffer-only `flush`, which left acknowledged writes in the page cache. The
/// framing helper's `Error::Index` is remapped to this module's error contract.
fn flush_wal(w: &mut BufWriter<std::fs::File>, wal_path: &Path) -> Result<()> {
    crate::index::wal_framing::flush_wal(w, wal_path, "sparse WAL")
        .map_err(|e| Error::SparseIndexError(format!("WAL flush/fsync failed: {e}")))
}

/// Computes the total byte length of an upsert WAL entry using checked arithmetic.
fn compute_upsert_entry_len(nnz: u32) -> Result<u32> {
    nnz.checked_mul(8)
        .and_then(|pairs_len| {
            1u32.checked_add(8)
                .and_then(|h| h.checked_add(4))
                .and_then(|h| h.checked_add(pairs_len))
        })
        .ok_or_else(|| {
            Error::SparseIndexError(format!(
                "WAL entry too large: nnz={nnz} would overflow u32 length prefix"
            ))
        })
}

/// Opens a WAL file for appending with buffered I/O.
fn open_wal_writer(wal_path: &Path) -> Result<BufWriter<std::fs::File>> {
    if let Some(generation) = committed_generation_for_wal(wal_path)? {
        align_wal_generation(wal_path, generation)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(wal_path)
        .map_err(|e| Error::SparseIndexError(format!("WAL open failed: {e}")))?;
    Ok(BufWriter::new(file))
}

/// Prevents post-compaction writes from extending a stale WAL generation.
fn align_wal_generation(wal_path: &Path, expected: u64) -> Result<()> {
    if read_wal_generation(wal_path)? != Some(expected) {
        reset_wal(wal_path, expected)?;
    }
    Ok(())
}

fn read_wal_generation(wal_path: &Path) -> Result<Option<u64>> {
    let file = match std::fs::File::open(wal_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Error::SparseIndexError(format!(
                "WAL generation read open failed: {error}"
            )))
        }
    };
    let mut bytes = Vec::with_capacity(WAL_HEADER_LEN);
    file.take(WAL_HEADER_LEN as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::SparseIndexError(format!("WAL generation read failed: {error}")))?;
    read_wal_header(&bytes).map(|header| header.map(|(generation, _)| generation))
}

/// Writes bytes to a WAL writer, mapping I/O errors to `SparseIndexError`.
fn wal_write(w: &mut BufWriter<std::fs::File>, bytes: &[u8]) -> Result<()> {
    w.write_all(bytes)
        .map_err(|e| Error::SparseIndexError(format!("WAL write failed: {e}")))
}

/// Appends a delete entry to the sparse WAL.
///
/// # Errors
///
/// Returns an error if the WAL file cannot be opened or written.
pub fn wal_append_delete(wal_path: &Path, point_id: u64) -> Result<()> {
    let total_len: u32 = 1 + 8;

    let mut w = open_wal_writer(wal_path)?;
    wal_write(&mut w, &total_len.to_le_bytes())?;
    wal_write(&mut w, &[WAL_OP_DELETE])?;
    wal_write(&mut w, &point_id.to_le_bytes())?;
    flush_wal(&mut w, wal_path)
}

// ---------------------------------------------------------------------------
// WAL replay
// ---------------------------------------------------------------------------

/// Replays a sparse WAL into the given index. Returns the number of entries replayed.
///
/// # Errors
///
/// Returns an error if the WAL file cannot be read or byte sequences are corrupt.
pub fn wal_replay(wal_path: &Path, index: &SparseInvertedIndex) -> Result<u64> {
    let data = read_wal_file(wal_path)?;
    let Some(data) = data else {
        return Ok(0);
    };
    let start = read_wal_header(&data)?.map_or(0, |(_, offset)| offset);
    replay_wal_bytes(&data, start, index)
}

pub(super) fn wal_replay_for_generation(
    wal_path: &Path,
    index: &SparseInvertedIndex,
    expected_generation: Option<u64>,
) -> Result<u64> {
    let Some(data) = read_wal_file(wal_path)? else {
        return Ok(0);
    };
    let header = read_wal_header(&data)?;
    let Some(expected) = expected_generation else {
        if header.is_some() {
            return Err(Error::SparseIndexError(
                "sparse WAL has a generation header but no snapshot manifest".to_string(),
            ));
        }
        return replay_wal_bytes(&data, 0, index);
    };
    match header {
        Some((generation, offset)) if generation == expected => {
            replay_wal_bytes(&data, offset, index)
        }
        Some(_) | None => Ok(0),
    }
}

pub(super) fn reset_wal(wal_path: &Path, generation: u64) -> Result<()> {
    atomic_write(wal_path, &wal_header(generation))
        .map_err(|e| Error::SparseIndexError(format!("sparse WAL durable reset: {e}")))
}

pub(super) fn sync_wal(wal_path: &Path) -> Result<()> {
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(wal_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::SparseIndexError(format!(
                "compact WAL sync open: {error}"
            )))
        }
    };
    file.sync_all()
        .and_then(|()| sync_parent_directory(wal_path))
        .map_err(|error| Error::SparseIndexError(format!("compact WAL sync: {error}")))
}

fn replay_wal_bytes(data: &[u8], mut pos: usize, index: &SparseInvertedIndex) -> Result<u64> {
    let mut count = 0_u64;

    while pos < data.len() {
        let Some((body_start, total_len)) = read_wal_entry_header(data, pos) else {
            break;
        };
        pos += 4;

        if pos + total_len > data.len() {
            tracing::warn!(
                "Sparse WAL truncated at offset {body_start}: declared {total_len} bytes but only {} remain",
                data.len() - pos
            );
            break;
        }

        let op = data[pos];
        pos += 1;

        let advanced = replay_single_entry(data, op, pos, body_start, total_len, index)?;
        if let Some((new_pos, counted)) = advanced {
            pos = new_pos;
            count += counted;
        } else {
            break;
        }

        advance_past_entry(&mut pos, body_start + total_len);
    }

    Ok(count)
}

fn wal_header(generation: u64) -> [u8; WAL_HEADER_LEN] {
    let mut header = [0_u8; WAL_HEADER_LEN];
    header[..4].copy_from_slice(&WAL_HEADER_MAGIC);
    header[4] = WAL_HEADER_VERSION;
    header[5..].copy_from_slice(&generation.to_le_bytes());
    header
}

fn read_wal_header(data: &[u8]) -> Result<Option<(u64, usize)>> {
    if !data.starts_with(&WAL_HEADER_MAGIC) {
        return Ok(None);
    }
    if data.len() < WAL_HEADER_LEN {
        return Err(Error::SparseIndexError(
            "sparse WAL generation header is truncated".to_string(),
        ));
    }
    if data[4] != WAL_HEADER_VERSION {
        return Err(Error::SparseIndexError(format!(
            "unsupported sparse WAL header version: {}",
            data[4]
        )));
    }
    let generation = read_le_u64(data, 5, "sparse WAL generation bytes")?;
    if generation == 0 {
        return Err(Error::SparseIndexError(
            "sparse WAL generation must be non-zero".to_string(),
        ));
    }
    Ok(Some((generation, WAL_HEADER_LEN)))
}

/// Reads the WAL file, returning `None` for missing files.
fn read_wal_file(wal_path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(wal_path) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::SparseIndexError(format!("WAL read failed: {e}"))),
    }
}

/// Replays a single WAL entry by opcode.
fn replay_single_entry(
    data: &[u8],
    op: u8,
    pos: usize,
    body_start: usize,
    total_len: usize,
    index: &SparseInvertedIndex,
) -> Result<Option<(usize, u64)>> {
    match op {
        WAL_OP_UPSERT => {
            let Some(new_pos) = replay_upsert_entry(data, pos, body_start, total_len, index)?
            else {
                return Ok(None);
            };
            Ok(Some((new_pos, 1)))
        }
        WAL_OP_DELETE => {
            let point_id = read_le_u64(data, pos, "WAL entry corrupted: bad point_id bytes")?;
            index.delete(point_id);
            Ok(Some((pos + 8, 1)))
        }
        unknown => {
            tracing::warn!("Sparse WAL unknown op 0x{unknown:02x} at offset {body_start}");
            Ok(Some((body_start + total_len, 0)))
        }
    }
}

/// Advances `pos` to at least `expected_end`.
fn advance_past_entry(pos: &mut usize, expected_end: usize) {
    if *pos < expected_end {
        *pos = expected_end;
    }
}

/// Reads the WAL entry length prefix.
fn read_wal_entry_header(data: &[u8], pos: usize) -> Option<(usize, usize)> {
    if pos + 4 > data.len() {
        tracing::warn!("Sparse WAL truncated at offset {pos}: not enough bytes for length prefix");
        return None;
    }
    let total_len =
        read_le_u32(data, pos, "WAL entry corrupted: bad length-prefix bytes").ok()? as usize;
    Some((pos + 4, total_len))
}

/// Replays a single upsert WAL entry.
fn replay_upsert_entry(
    data: &[u8],
    mut pos: usize,
    body_start: usize,
    total_len: usize,
    index: &SparseInvertedIndex,
) -> Result<Option<usize>> {
    if total_len < 1 + 8 + 4 {
        tracing::warn!("Sparse WAL upsert entry too short at offset {body_start}");
        return Ok(None);
    }
    let point_id = read_le_u64(data, pos, "WAL entry corrupted: bad point_id bytes")?;
    pos += 8;
    let nnz = read_le_u32(data, pos, "WAL entry corrupted: bad nnz bytes")? as usize;
    pos += 4;

    // Untrusted `nnz`: each pair is 8 bytes (u32 term_id + f32 weight). Compute
    // in u64 with checked/saturating arithmetic so a crafted `nnz` cannot wrap on
    // a 32-bit target and defeat the truncation guard below. `read_term_weight_pairs`
    // then bounds its own `Vec::with_capacity` against the validated `nnz`.
    let Some(payload_bytes) = (nnz as u64).checked_mul(8) else {
        tracing::warn!("Sparse WAL upsert entry nnz overflow at offset {body_start}");
        return Ok(None);
    };
    let entry_end = (body_start as u64).saturating_add(total_len as u64);
    if entry_end < (pos as u64).saturating_add(payload_bytes) {
        tracing::warn!("Sparse WAL upsert entry truncated at offset {body_start}");
        return Ok(None);
    }

    let pairs = read_term_weight_pairs(data, &mut pos, nnz)?;
    let vector = SparseVector::new(pairs);
    index.insert(point_id, &vector);
    Ok(Some(pos))
}

/// Reads `nnz` (`term_id`, weight) pairs from the data buffer.
fn read_term_weight_pairs(data: &[u8], pos: &mut usize, nnz: usize) -> Result<Vec<(u32, f32)>> {
    // Defensive cap: a pair is 8 bytes, so the data buffer can hold at most
    // `data.len() / 8` pairs. Clamp the pre-allocation so an untrusted `nnz`
    // cannot pre-reserve far beyond what the buffer could ever supply.
    let mut pairs = Vec::with_capacity(nnz.min(data.len() / 8));
    for _ in 0..nnz {
        let idx = read_le_u32(data, *pos, "WAL entry corrupted: bad term-index bytes")?;
        *pos += 4;
        let val = read_le_f32(data, *pos, "WAL entry corrupted: bad weight bytes")?;
        *pos += 4;
        pairs.push((idx, val));
    }
    Ok(pairs)
}
