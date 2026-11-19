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

/// Flushes the WAL writer's buffer to the file.
///
/// Every WAL flush whose failure must resync the write offset goes through
/// here, so a test can fail it (`fail_next_wal_flush`, test builds only). The injected
/// failure returns before writing anything, which leaves the buffer exactly
/// as a flush that wrote nothing does.
pub(super) fn flush_wal(wal: &mut io::BufWriter<std::fs::File>) -> io::Result<()> {
    #[cfg(test)]
    if FAIL_NEXT_WAL_FLUSH.with(|fail| fail.replace(false)) {
        return Err(io::Error::other("injected WAL flush failure"));
    }
    wal.flush()
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_WAL_FLUSH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEAR_NEXT_WAL_APPEND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes the next [`flush_wal`] on this thread fail; the guard clears the
/// fault on drop if no flush consumed it.
#[cfg(test)]
pub(super) fn fail_next_wal_flush() -> WalFault {
    WalFault::arm(&FAIL_NEXT_WAL_FLUSH)
}

/// Makes the next [`append_record`] on this thread fail after writing half
/// the record to the file, the state an inner `write_all` that hits ENOSPC
/// midway leaves (#2850); the guard clears the fault on drop if no append
/// consumed it.
#[cfg(test)]
pub(super) fn tear_next_wal_append() -> WalFault {
    WalFault::arm(&TEAR_NEXT_WAL_APPEND)
}

/// An armed one-shot WAL fault, cleared on drop if nothing consumed it.
#[cfg(test)]
pub(super) struct WalFault(&'static std::thread::LocalKey<std::cell::Cell<bool>>);

#[cfg(test)]
impl WalFault {
    fn arm(fault: &'static std::thread::LocalKey<std::cell::Cell<bool>>) -> Self {
        fault.with(|armed| armed.set(true));
        Self(fault)
    }
}

#[cfg(test)]
impl Drop for WalFault {
    fn drop(&mut self) {
        self.0.with(|armed| armed.set(false));
    }
}

/// The WAL offset the next record will land at, after a failed write or flush.
///
/// The WAL file is opened in append mode, and a `BufWriter` whose flush fails
/// keeps the bytes it could not write and writes them on the next successful
/// flush. So the next record lands after the file *and* that retained buffer —
/// resyncing to the file length alone made every later index entry point
/// that many bytes too early.
pub(super) fn wal_end<W: Write>(wal: &io::BufWriter<W>, file_len: u64) -> u64 {
    file_len.saturating_add(wal.buffer().len() as u64)
}

/// Appends one complete WAL record, or leaves the WAL as if it never started.
///
/// `offset` is the tracked WAL offset (`write_offset`) before this record:
/// the file length plus the bytes the writer still buffers, which every
/// write path keeps true. A failed `write_all` can leave a fragment of the
/// record in the file — a record at least as large as the buffer goes
/// straight to the file, which may take part of it before failing. Left
/// there, the fragment is a torn record in the middle of the WAL once a
/// later record lands after it: replay stops at it and `truncate_torn_tail`
/// discards every record behind it, acknowledged ones included (#2850). So
/// the fragment is cut off before the error is returned.
///
/// If the truncation itself fails, the fragment stays and the double fault
/// is logged as an error. `offset` is then resynced past the fragment
/// ([`wal_end`]), so later records still read back until the store is
/// reopened; replay will stop at the fragment on that open.
pub(super) fn append_record(
    wal: &mut io::BufWriter<std::fs::File>,
    record: &[u8],
    offset: &mut u64,
) -> io::Result<()> {
    write_record(wal, record).inspect_err(|_| {
        if let Err(error) = discard_torn_record(wal.get_ref(), wal.buffer().len(), *offset) {
            tracing::error!(
                %error,
                "payload WAL torn append could not be truncated; replay will stop at it on the next open"
            );
            if let Ok(meta) = wal.get_ref().metadata() {
                *offset = wal_end(wal, meta.len());
            }
        }
    })
}

fn write_record(wal: &mut io::BufWriter<std::fs::File>, record: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    if TEAR_NEXT_WAL_APPEND.with(|armed| armed.replace(false)) {
        // What `BufWriter::write_all` does with a record it cannot buffer:
        // flush, then write straight to the file — here, only half of it.
        wal.flush()?;
        wal.get_mut().write_all(&record[..record.len() / 2])?;
        return Err(io::Error::other("injected torn WAL append"));
    }
    wal.write_all(record)
}

/// Truncates `file` to `offset` minus the `buffered` bytes the writer still
/// holds, dropping any fragment a failed append wrote. A failure that left
/// only buffered bytes (a partial flush of earlier, complete records) finds
/// the file already at that length and changes nothing.
pub(super) fn discard_torn_record(
    file: &std::fs::File,
    buffered: usize,
    offset: u64,
) -> io::Result<()> {
    let file_end = offset.saturating_sub(buffered as u64);
    let file_len = file.metadata()?.len();
    if file_len > file_end {
        file.set_len(file_end)?;
        tracing::warn!(
            discarded_bytes = file_len - file_end,
            "payload WAL torn append truncated"
        );
    }
    Ok(())
}

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

    append_record(wal, record_buf, offset)?;

    let bytes_written = 1 + 8 + 4 + u64::from(len_u32) + 4;
    *offset += bytes_written;
    // Marker(1) + ID(8) = 9 bytes before the length field
    index.insert(id, record_start + 9);

    Ok(())
}
