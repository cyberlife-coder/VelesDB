//! WAL replay for `MmapStorage` crash recovery.
//!
//! Issue #317: On crash, WAL data written since the last `flush()` would be
//! lost because the index file (`vectors.idx`) only reflects the last
//! flushed state. This module replays WAL entries to recover those writes.
//!
//! # WAL Format (CRC32-framed, Issue #317)
//!
//! ```text
//! Store:  [op=1: 1B] [id: 8B LE] [len: 4B LE] [data: len B] [crc32: 4B LE]
//! Delete: [op=2: 1B] [id: 8B LE] [crc32: 4B LE]
//! ```
//!
//! # Legacy WAL Format (pre-#317, no CRC)
//!
//! Detected by validating the CRC of the first entry. If validation fails,
//! the file is legacy format and replay is skipped (the persisted index is
//! authoritative for legacy data).

use crate::storage::log_payload::crc32_hash;
use crate::storage::sharded_index::ShardedIndex;

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek};
use std::path::Path;

/// Minimum store entry size: op(1) + id(8) + len(4) + crc(4) = 17.
const MIN_STORE_ENTRY: usize = 17;
/// Delete entry size: op(1) + id(8) + crc(4) = 13.
const DELETE_ENTRY_SIZE: usize = 13;

/// Replays CRC32-framed WAL entries into the sharded index and mmap.
///
/// Skips legacy (non-CRC) WAL files. Truncates the WAL after a
/// successful replay so entries are not replayed twice.
///
/// # Returns
///
/// Number of WAL entries successfully replayed.
#[allow(clippy::module_name_repetitions)]
pub(crate) fn replay_wal_to_index(
    wal_path: &Path,
    index: &ShardedIndex,
    dimension: usize,
    mmap_data: &mut [u8],
    next_offset: &mut usize,
) -> io::Result<usize> {
    let Some((mut reader, file_len)) = open_crc_wal(wal_path)? else {
        return Ok(0);
    };

    let vector_size = dimension * std::mem::size_of::<f32>();
    let replayed = drain_wal_entries(
        &mut reader,
        file_len,
        index,
        mmap_data,
        next_offset,
        vector_size,
    );

    if replayed > 0 {
        truncate_wal(wal_path)?;
    }

    Ok(replayed)
}

/// Opens the WAL file and validates it uses CRC32-framed format.
///
/// Returns `None` if the file is missing, empty, or uses the legacy format.
fn open_crc_wal(wal_path: &Path) -> io::Result<Option<(BufReader<File>, u64)>> {
    if !wal_path.exists() {
        return Ok(None);
    }

    let file = File::open(wal_path)?;
    let file_len = file.metadata()?.len();
    if file_len == 0 {
        return Ok(None);
    }

    if !is_crc_framed_wal(wal_path, file_len)? {
        return Ok(None);
    }

    let reader = BufReader::new(File::open(wal_path)?);
    Ok(Some((reader, file_len)))
}

/// Replays all valid entries from the WAL, returning the count.
fn drain_wal_entries(
    reader: &mut BufReader<File>,
    file_len: u64,
    index: &ShardedIndex,
    mmap_data: &mut [u8],
    next_offset: &mut usize,
    vector_size: usize,
) -> usize {
    let mut replayed = 0usize;
    while let Ok(true) =
        replay_one_entry(reader, file_len, index, mmap_data, next_offset, vector_size)
    {
        replayed += 1;
    }
    replayed
}

// ---------------------------------------------------------------------------
// Format Detection
// ---------------------------------------------------------------------------

/// Returns `true` if the WAL uses CRC32-framed entries.
fn is_crc_framed_wal(wal_path: &Path, file_len: u64) -> io::Result<bool> {
    let min_size = MIN_STORE_ENTRY.min(DELETE_ENTRY_SIZE) as u64;
    if file_len < min_size {
        return Ok(false);
    }

    let mut reader = BufReader::new(File::open(wal_path)?);
    let mut op = [0u8; 1];
    if reader.read_exact(&mut op).is_err() {
        return Ok(false);
    }

    match op[0] {
        1 => validate_first_store_crc(&mut reader),
        2 => Ok(validate_first_delete_crc(&mut reader)),
        _ => Ok(false),
    }
}

/// Validates the CRC of the first store entry.
fn validate_first_store_crc(reader: &mut BufReader<File>) -> io::Result<bool> {
    let mut id_bytes = [0u8; 8];
    let mut len_bytes = [0u8; 4];
    if reader.read_exact(&mut id_bytes).is_err() || reader.read_exact(&mut len_bytes).is_err() {
        return Ok(false);
    }

    let data_len = usize::try_from(u32::from_le_bytes(len_bytes))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "data_len overflow"))?;

    let mut data = vec![0u8; data_len];
    if reader.read_exact(&mut data).is_err() {
        return Ok(false);
    }

    let mut stored_crc = [0u8; 4];
    if reader.read_exact(&mut stored_crc).is_err() {
        return Ok(false);
    }

    let mut frame = Vec::with_capacity(1 + 8 + 4 + data_len);
    frame.push(1u8);
    frame.extend_from_slice(&id_bytes);
    frame.extend_from_slice(&len_bytes);
    frame.extend_from_slice(&data);

    Ok(crc32_hash(&frame) == u32::from_le_bytes(stored_crc))
}

/// Validates the CRC of the first delete entry.
fn validate_first_delete_crc(reader: &mut BufReader<File>) -> bool {
    let mut id_bytes = [0u8; 8];
    if reader.read_exact(&mut id_bytes).is_err() {
        return false;
    }

    let mut stored_crc = [0u8; 4];
    if reader.read_exact(&mut stored_crc).is_err() {
        return false;
    }

    let mut frame = Vec::with_capacity(1 + 8);
    frame.push(2u8);
    frame.extend_from_slice(&id_bytes);

    crc32_hash(&frame) == u32::from_le_bytes(stored_crc)
}

// ---------------------------------------------------------------------------
// Entry Replay
// ---------------------------------------------------------------------------

/// Replays one WAL entry. Returns `Ok(true)` on success, `Ok(false)` at EOF.
fn replay_one_entry(
    reader: &mut BufReader<File>,
    file_len: u64,
    index: &ShardedIndex,
    mmap_data: &mut [u8],
    next_offset: &mut usize,
    vector_size: usize,
) -> io::Result<bool> {
    if reader.stream_position()? >= file_len {
        return Ok(false);
    }

    let mut op = [0u8; 1];
    if reader.read_exact(&mut op).is_err() {
        return Ok(false);
    }

    match op[0] {
        1 => replay_store(reader, index, mmap_data, next_offset, vector_size),
        2 => replay_delete(reader, index),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "unknown WAL op")),
    }
}

/// Reads and CRC-validates a store entry from the WAL.
///
/// Returns `(id, data)` on success.
fn read_store_entry(reader: &mut BufReader<File>) -> io::Result<(u64, Vec<u8>)> {
    let mut id_bytes = [0u8; 8];
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut id_bytes)?;
    reader.read_exact(&mut len_bytes)?;

    let data_len = usize::try_from(u32::from_le_bytes(len_bytes))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "data_len overflow"))?;

    let mut data = vec![0u8; data_len];
    reader.read_exact(&mut data)?;

    let mut stored_crc = [0u8; 4];
    reader.read_exact(&mut stored_crc)?;

    verify_store_crc(id_bytes, len_bytes, &data, stored_crc)?;

    Ok((u64::from_le_bytes(id_bytes), data))
}

/// Verifies the CRC32 of a store WAL frame.
fn verify_store_crc(
    id_bytes: [u8; 8],
    len_bytes: [u8; 4],
    data: &[u8],
    stored_crc: [u8; 4],
) -> io::Result<()> {
    let mut frame = Vec::with_capacity(1 + 8 + 4 + data.len());
    frame.push(1u8);
    frame.extend_from_slice(&id_bytes);
    frame.extend_from_slice(&len_bytes);
    frame.extend_from_slice(data);

    if crc32_hash(&frame) == u32::from_le_bytes(stored_crc) {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "CRC mismatch"))
    }
}

/// Replays a store entry: validates CRC, writes data to mmap, updates index.
fn replay_store(
    reader: &mut BufReader<File>,
    index: &ShardedIndex,
    mmap_data: &mut [u8],
    next_offset: &mut usize,
    vector_size: usize,
) -> io::Result<bool> {
    let (id, data) = read_store_entry(reader)?;

    if data.len() == vector_size {
        apply_store_to_mmap(id, &data, index, mmap_data, next_offset, vector_size);
    }

    Ok(true)
}

/// Writes vector data into the mmap region and updates the index.
fn apply_store_to_mmap(
    id: u64,
    data: &[u8],
    index: &ShardedIndex,
    mmap_data: &mut [u8],
    next_offset: &mut usize,
    vector_size: usize,
) {
    let offset = index.get(id).unwrap_or_else(|| {
        let off = *next_offset;
        *next_offset += vector_size;
        off
    });

    let end = offset + vector_size;
    if end <= mmap_data.len() {
        mmap_data[offset..end].copy_from_slice(data);
        index.insert(id, offset);
    } else {
        tracing::warn!(
            id = id,
            offset = offset,
            end = end,
            mmap_len = mmap_data.len(),
            "WAL replay: skipping vector — mmap too small"
        );
    }
}

/// Replays a delete entry: validates CRC, removes id from index.
fn replay_delete(reader: &mut BufReader<File>, index: &ShardedIndex) -> io::Result<bool> {
    let mut id_bytes = [0u8; 8];
    reader.read_exact(&mut id_bytes)?;

    let mut stored_crc = [0u8; 4];
    reader.read_exact(&mut stored_crc)?;

    // Validate CRC
    let mut frame = Vec::with_capacity(1 + 8);
    frame.push(2u8);
    frame.extend_from_slice(&id_bytes);

    if crc32_hash(&frame) != u32::from_le_bytes(stored_crc) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "CRC mismatch"));
    }

    let id = u64::from_le_bytes(id_bytes);
    index.remove(id);
    Ok(true)
}

/// Truncates WAL after successful replay.
fn truncate_wal(wal_path: &Path) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(wal_path)?;
    file.set_len(0)?;
    file.sync_all()
}
