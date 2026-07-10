//! File-backed [`WalCursor`] over the existing append-only WAL framing.
//!
//! [`LogWalCursor`] is a concrete, read-only cursor that reads the same
//! `payloads.log` marker/CRC framing written by
//! [`LogPayloadStorage`](super::log_payload::LogPayloadStorage). It walks the
//! log forward, yielding one [`WalRecord`] per framed entry, so a replication
//! consumer can identify a stable position and stream entries from it
//! (Requirement 6.2).
//!
//! # On-disk format
//!
//! This is a pure read API over the existing framing (see
//! [`super::wal_entry`] and [`super::log_payload_io`]). It writes nothing and
//! introduces no on-disk change; legacy (no-CRC) and current (CRC) frames are
//! both read (Requirement 6.4). A torn tail — a final frame truncated by a
//! crash mid-append — is skipped and never yielded, matching the sequential
//! replay policy in [`super::wal_entry`].
//!
//! # Boundary
//!
//! Core exposes this; premium consumes it. Core never depends on premium
//! (Requirement 6.5). Consumer low-watermark retention is delegated to an
//! embedded [`WalWatermarkRegistry`].

use super::log_payload_io::{
    CRC_DELETE_MARKER, CRC_STORE_MARKER, LEGACY_DELETE_MARKER, LEGACY_STORE_MARKER,
};
use super::wal_cursor::{WalConsumerId, WalCursor, WalPosition, WalRecord, WalWatermarkRegistry};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Framed body length (excluding the 1-byte marker) of a legacy delete record:
/// `id(8)`.
const DELETE_LEGACY_BODY: u64 = 8;
/// Framed body length (excluding the 1-byte marker) of a CRC delete record:
/// `id(8) + crc(4)`.
const DELETE_CRC_BODY: u64 = 8 + 4;

/// A concrete, file-backed [`WalCursor`] over a collection's `payloads.log`.
///
/// Reads the existing marker/CRC framing forward from a stable
/// [`WalPosition`]. The cursor holds only the path to the log and in-memory
/// consumer-watermark state; it opens the file per read so it always observes
/// the current durable tail.
#[derive(Debug)]
pub struct LogWalCursor {
    /// Path to the append-only `payloads.log` WAL file.
    log_path: PathBuf,
    /// In-memory low-watermark registration state (Requirement 6.3).
    registry: WalWatermarkRegistry,
}

impl LogWalCursor {
    /// Creates a cursor over the `payloads.log` inside `dir`.
    ///
    /// `dir` is the storage directory used by
    /// [`LogPayloadStorage`](super::log_payload::LogPayloadStorage); the cursor
    /// reads the `payloads.log` file within it.
    #[must_use]
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            log_path: dir.as_ref().join("payloads.log"),
            registry: WalWatermarkRegistry::new(),
        }
    }

    /// The current durable WAL length in bytes (0 if the file is absent).
    fn durable_len(&self) -> u64 {
        std::fs::metadata(&self.log_path).map_or(0, |m| m.len())
    }

    /// The minimum retained low-watermark across all registered consumers, or
    /// `None` when no consumer is registered (no retention hold).
    #[must_use]
    pub fn min_watermark(&self) -> Option<WalPosition> {
        self.registry.min_watermark()
    }
}

impl WalCursor for LogWalCursor {
    fn read_from(&self, from: WalPosition, max: usize) -> crate::Result<Vec<WalRecord>> {
        if max == 0 {
            return Ok(Vec::new());
        }
        let durable_len = self.durable_len();
        let mut pos = from.offset();
        if pos >= durable_len {
            return Ok(Vec::new());
        }

        let mut file = File::open(&self.log_path)?;
        let mut out = Vec::new();
        while out.len() < max && pos < durable_len {
            let Some(record) = read_one_frame(&mut file, pos, durable_len)? else {
                break; // torn tail or unknown marker: stop cleanly
            };
            pos = record.next.offset();
            out.push(record);
        }
        Ok(out)
    }

    fn tail_position(&self) -> WalPosition {
        WalPosition::new(self.durable_len())
    }

    fn register_consumer(&self) -> WalConsumerId {
        self.registry.register()
    }

    fn deregister_consumer(&self, consumer: WalConsumerId) {
        self.registry.deregister(consumer);
    }

    fn advance_low_watermark(&self, consumer: WalConsumerId, up_to: WalPosition) {
        self.registry.advance(consumer, up_to);
    }
}

/// Reads one framed record starting at `pos`.
///
/// Returns `Ok(None)` for a torn tail (a frame extending past `durable_len`),
/// EOF, or an unknown marker — the caller stops cleanly. Otherwise returns the
/// record with its raw framed bytes, its `position`, and the `next` cursor.
fn read_one_frame(file: &mut File, pos: u64, durable_len: u64) -> io::Result<Option<WalRecord>> {
    file.seek(SeekFrom::Start(pos))?;
    let mut marker = [0u8; 1];
    if file.read_exact(&mut marker).is_err() {
        return Ok(None);
    }
    let Some(body_len) = frame_body_len(file, marker[0]) else {
        return Ok(None);
    };
    let total = 1 + body_len;
    if pos.saturating_add(total) > durable_len {
        return Ok(None); // torn tail
    }
    let bytes = read_frame_bytes(file, pos, total)?;
    Ok(Some(WalRecord {
        position: WalPosition::new(pos),
        next: WalPosition::new(pos + total),
        bytes,
    }))
}

/// Computes the framed body length (bytes after the 1-byte marker) for the
/// given marker, reading the length field for store records. The file cursor
/// is positioned immediately after the marker. Returns `None` for an unknown
/// marker or a header truncated by a torn tail.
fn frame_body_len(file: &mut File, marker: u8) -> Option<u64> {
    match marker {
        LEGACY_STORE_MARKER => store_body_len(file, false),
        CRC_STORE_MARKER => store_body_len(file, true),
        LEGACY_DELETE_MARKER => Some(DELETE_LEGACY_BODY),
        CRC_DELETE_MARKER => Some(DELETE_CRC_BODY),
        _ => None,
    }
}

/// Reads a store record's body length: `id(8) + len(4) + payload(len)` plus a
/// trailing `crc(4)` when `has_crc`. The cursor is positioned right after the
/// marker. Returns `None` if the header is truncated (torn tail).
fn store_body_len(file: &mut File, has_crc: bool) -> Option<u64> {
    let mut id_bytes = [0u8; 8];
    if file.read_exact(&mut id_bytes).is_err() {
        return None;
    }
    let mut len_bytes = [0u8; 4];
    if file.read_exact(&mut len_bytes).is_err() {
        return None;
    }
    let payload_len = u64::from(u32::from_le_bytes(len_bytes));
    let crc_len = if has_crc { 4 } else { 0 };
    Some(8 + 4 + payload_len + crc_len)
}

/// Reads exactly `total` bytes of a framed record starting at `pos`.
fn read_frame_bytes(file: &mut File, pos: u64, total: u64) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(pos))?;
    let cap = usize::try_from(total)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "WAL frame too large"))?;
    let mut bytes = vec![0u8; cap];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
