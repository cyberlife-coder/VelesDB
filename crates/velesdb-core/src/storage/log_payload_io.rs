//! WAL record I/O helpers for log-structured payload storage.
//!
//! Contains WAL format markers, CRC computation, and record serialization.
//! Extracted from `log_payload.rs` to keep file NLOC within limits.

use super::snapshot::crc32_hash;

use rustc_hash::FxHashMap;
use std::io::{self, Write};

// ---------------------------------------------------------------------------
// WAL format markers
// ---------------------------------------------------------------------------

/// Legacy WAL store marker (no CRC).
pub(super) const LEGACY_STORE_MARKER: u8 = 1;
/// Legacy WAL delete marker (no CRC).
pub(super) const LEGACY_DELETE_MARKER: u8 = 2;
/// CRC32-protected store marker.
pub(super) const CRC_STORE_MARKER: u8 = 0xC3;
/// CRC32-protected delete marker.
pub(super) const CRC_DELETE_MARKER: u8 = 0xC4;

// ---------------------------------------------------------------------------
// CRC32 helpers
// ---------------------------------------------------------------------------

/// Computes CRC32 for a WAL store record (marker + id + len + payload).
///
/// # Panics
///
/// Panics if `payload.len()` exceeds `u32::MAX`. Callers must validate length first.
pub(super) fn compute_store_crc(id: u64, payload: &[u8]) -> u32 {
    // Reason: caller validates payload fits in u32 before calling (store validates
    // via try_from, replay reads a u32 length field).
    #[allow(clippy::cast_possible_truncation)]
    let len_u32 = payload.len() as u32;
    let mut buf = Vec::with_capacity(1 + 8 + 4 + payload.len());
    buf.push(CRC_STORE_MARKER);
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&len_u32.to_le_bytes());
    buf.extend_from_slice(payload);
    crc32_hash(&buf)
}

/// Computes CRC32 for a WAL delete record (marker + id).
pub(super) fn compute_delete_crc(id: u64) -> u32 {
    let mut buf = [0u8; 1 + 8];
    buf[0] = CRC_DELETE_MARKER;
    buf[1..9].copy_from_slice(&id.to_le_bytes());
    crc32_hash(&buf)
}

// ---------------------------------------------------------------------------
// WAL record writing
// ---------------------------------------------------------------------------

/// Serializes a payload and writes a CRC-protected WAL store record.
///
/// Shared by `store()` (per-point) and `store_batch()` (batched) to avoid
/// duplicating the record-building logic.
///
/// Reuses `record_buf` to avoid per-call heap allocation in batch mode.
pub(super) fn write_store_record(
    wal: &mut io::BufWriter<std::fs::File>,
    id: u64,
    payload: &serde_json::Value,
    offset: &mut u64,
    index: &mut FxHashMap<u64, u64>,
    record_buf: &mut Vec<u8>,
) -> io::Result<()> {
    let record_start = *offset;

    // Header: Marker(0xC3) | ID(8) | Len placeholder(4)
    record_buf.clear();
    record_buf.push(CRC_STORE_MARKER);
    record_buf.extend_from_slice(&id.to_le_bytes());
    let len_pos = record_buf.len();
    record_buf.extend_from_slice(&0u32.to_le_bytes());

    // Serialize directly into record_buf — zero intermediate allocation
    let payload_start = record_buf.len();
    serde_json::to_writer(&mut *record_buf, payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let payload_len = record_buf.len() - payload_start;

    // Patch length field now that we know the serialized size
    let len_u32 = u32::try_from(payload_len)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Payload too large"))?;
    record_buf[len_pos..len_pos + 4].copy_from_slice(&len_u32.to_le_bytes());

    // CRC over the record prefix (everything before the CRC field)
    let crc = crc32_hash(record_buf);
    record_buf.extend_from_slice(&crc.to_le_bytes());

    wal.write_all(record_buf)?;

    let bytes_written = 1 + 8 + 4 + u64::from(len_u32) + 4;
    *offset += bytes_written;
    // Marker(1) + ID(8) = 9 bytes before the length field
    index.insert(id, record_start + 9);

    Ok(())
}
