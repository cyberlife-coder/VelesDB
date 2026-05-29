//! Snapshot persistence for log-structured payload storage.
//!
//! Handles saving and loading binary snapshots of the in-memory payload index
//! for O(1) cold-start recovery instead of O(N) WAL replay.
//!
//! ## Snapshot Format
//!
//! ```text
//! [Magic: "VSNP" 4 bytes]
//! [Version: 1 byte]
//! [WAL position: 8 bytes]
//! [Entry count: 8 bytes]
//! [Entries: (id: u64, offset: u64) × N]
//! [CRC32: 4 bytes]
//! ```

use rustc_hash::FxHashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// Snapshot file magic bytes.
pub(crate) const SNAPSHOT_MAGIC: &[u8; 4] = b"VSNP";

/// Current snapshot format version.
pub(crate) const SNAPSHOT_VERSION: u8 = 1;

/// Default threshold for automatic snapshot creation (10 MB of WAL since last snapshot).
pub(crate) const DEFAULT_SNAPSHOT_THRESHOLD: u64 = 10 * 1024 * 1024;

/// Simple CRC32 implementation (IEEE 802.3 polynomial).
///
/// Used for snapshot integrity validation.
#[inline]
#[allow(clippy::cast_possible_truncation)] // Table index always 0-255
pub(crate) fn crc32_hash(data: &[u8]) -> u32 {
    const CRC32_TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut j = 0;
            while j < 8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB8_8320;
                } else {
                    crc >>= 1;
                }
                j += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };

    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

/// Loads index from a snapshot file.
///
/// Returns `(index, wal_position)` if successful.
///
/// # Errors
///
/// Returns an error if the snapshot file is missing, corrupt, or has an invalid format.
/// Validates magic, version, and minimum size of snapshot data.
fn validate_snapshot_format(data: &[u8]) -> io::Result<()> {
    if data.len() < 25 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Snapshot too small",
        ));
    }
    if &data[0..4] != SNAPSHOT_MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid magic"));
    }
    if data[4] != SNAPSHOT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unsupported version",
        ));
    }
    Ok(())
}

/// Validates snapshot CRC and entry count, returns (wal_pos, entry_count).
fn validate_snapshot_header(data: &[u8]) -> io::Result<(u64, usize)> {
    validate_snapshot_format(data)?;

    let wal_pos = u64::from_le_bytes(
        data[5..13]
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid WAL position"))?,
    );
    let entry_count_u64 = u64::from_le_bytes(
        data[13..21]
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid entry count"))?,
    );

    // P1 Audit: Validate entry_count BEFORE conversion to prevent DoS
    let max_possible_entries = data.len().saturating_sub(25) / 16;
    if entry_count_u64 > max_possible_entries as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Entry count exceeds data size",
        ));
    }
    #[allow(clippy::cast_possible_truncation)] // Validated above
    let entry_count = entry_count_u64 as usize;

    if data.len() != 21 + entry_count * 16 + 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Size mismatch"));
    }

    let stored_crc = u32::from_le_bytes(
        data[data.len() - 4..]
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid CRC"))?,
    );
    if stored_crc != crc32_hash(&data[..data.len() - 4]) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "CRC mismatch"));
    }

    Ok((wal_pos, entry_count))
}

pub(crate) fn load_snapshot(snapshot_path: &Path) -> io::Result<(FxHashMap<u64, u64>, u64)> {
    if !snapshot_path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "No snapshot"));
    }

    let data = std::fs::read(snapshot_path)?;
    let (wal_pos, entry_count) = validate_snapshot_header(&data)?;

    let mut index = FxHashMap::default();
    index.reserve(entry_count);

    let entries_start = 21;
    for i in 0..entry_count {
        let offset = entries_start + i * 16;
        let id = u64::from_le_bytes(
            data[offset..offset + 8]
                .try_into()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid entry ID"))?,
        );
        let wal_offset = u64::from_le_bytes(
            data[offset + 8..offset + 16]
                .try_into()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid entry offset"))?,
        );
        index.insert(id, wal_offset);
    }

    Ok((index, wal_pos))
}

/// Creates a snapshot file from the given index state.
///
/// Writes atomically via temp file + fsync + rename.
///
/// # Errors
///
/// Returns an error if file operations fail.
pub(crate) fn create_snapshot_file(
    dir: &Path,
    index: &FxHashMap<u64, u64>,
    wal_pos: u64,
) -> io::Result<()> {
    let snapshot_path = dir.join("payloads.snapshot");

    // Calculate buffer size
    let entry_count = index.len();
    let buf_size = 21 + entry_count * 16 + 4; // header + entries + crc
    let mut buf = Vec::with_capacity(buf_size);

    // Write header
    buf.extend_from_slice(SNAPSHOT_MAGIC);
    buf.push(SNAPSHOT_VERSION);
    buf.extend_from_slice(&wal_pos.to_le_bytes());
    buf.extend_from_slice(&(entry_count as u64).to_le_bytes());

    // Write entries
    for (&id, &offset) in index {
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
    }

    // Compute and append CRC
    let crc = crc32_hash(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());

    // Write atomically via temp file + fsync + rename
    let temp_path = dir.join("payloads.snapshot.tmp");
    {
        let file = File::create(&temp_path)?;
        let mut writer = io::BufWriter::new(file);
        writer.write_all(&buf)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
    }
    std::fs::rename(&temp_path, &snapshot_path)?;

    // #898: fsync the parent directory so the rename itself is durable. Without
    // this, a power loss after rename can leave the directory entry pointing at
    // the old (or no) snapshot even though the file contents were fsynced.
    sync_parent_dir(dir);

    Ok(())
}

/// Best-effort fsync of a directory so a preceding rename/create is durable.
///
/// Errors are intentionally ignored: not all platforms permit opening or
/// syncing a directory handle, and the snapshot is still recoverable via WAL
/// replay if the directory entry is lost.
fn sync_parent_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

/// Returns whether a new snapshot should be created based on WAL growth.
///
/// Returns true if WAL has grown by more than [`DEFAULT_SNAPSHOT_THRESHOLD`]
/// bytes since the last snapshot.
#[must_use]
pub(crate) fn should_create_snapshot(last_snapshot_pos: u64, current_pos: u64) -> bool {
    current_pos.saturating_sub(last_snapshot_pos) >= DEFAULT_SNAPSHOT_THRESHOLD
}
