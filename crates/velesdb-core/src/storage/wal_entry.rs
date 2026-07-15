//! WAL entry domain type — separates parsing from application.
//!
//! Extracted from `log_payload.rs` to reduce NLOC.
//!
//! ## WAL versioning (WP-2I)
//!
//! The WAL format is implicitly versioned through its marker bytes. Legacy
//! entries use `LEGACY_STORE_MARKER` / `LEGACY_DELETE_MARKER` (no CRC),
//! while v2 entries use `CRC_STORE_MARKER` / `CRC_DELETE_MARKER` (with
//! CRC32 integrity checking). Future format changes should introduce new
//! marker values, preserving backward-compatible reading of older entries.
//! No separate schema-version header is needed because the per-entry
//! marker already encodes the wire format.
//!
//! ## Torn-tail and corruption policy
//!
//! A crash mid-append leaves a partially-written final record. Because the WAL
//! is append-only and replayed sequentially, a truncation can only ever be the
//! last record. [`WalEntry::read`] signals such a tail by returning `None`, and
//! [`WalEntry::apply`] by returning `Ok(None)`, so the caller stops replay
//! cleanly and keeps every prior entry rather than failing the collection open.
//!
//! The CRC only ever covers the id + payload bytes; it never protects the
//! framing fields (marker and length) that are read *before* any CRC check. A
//! flipped marker byte, or a shrunk length field that slips past the OOM guard
//! and desynchronises the framing of the following record, therefore surfaces
//! as an *unknown marker* mid-stream. [`WalEntry::read`] treats that exactly
//! like a torn tail — it returns `None` so the caller stops and preserves every
//! already-replayed entry rather than failing the collection open — and
//! additionally records the shared `wal_replay_corrupt_entries` metric plus a
//! warning so the corruption is observable. This mirrors the vector WAL's
//! `#898` policy (`storage::mmap::wal_replay`), which likewise stops on an
//! unknown opcode and records the same metric.

use super::log_payload_io::{
    compute_delete_crc, compute_store_crc, CRC_DELETE_MARKER, CRC_STORE_MARKER,
    LEGACY_DELETE_MARKER, LEGACY_STORE_MARKER,
};
use rustc_hash::FxHashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};

/// A parsed WAL entry with its file position context.
pub(super) struct WalEntry {
    op: WalOp,
    /// File position after the marker + ID header.
    pos_after_header: u64,
    /// Whether this entry uses CRC32 integrity checking.
    has_crc: bool,
}

/// The two WAL operations: store (upsert) or delete.
enum WalOp {
    Store { id: u64 },
    Delete { id: u64 },
}

impl WalEntry {
    /// Reads one WAL entry from the reader. Returns `None` to stop replay
    /// cleanly — on EOF, on a torn tail (truncated header), or on an unknown
    /// marker (mid-stream corruption); see the module-level policy. The caller
    /// keeps every entry already read.
    ///
    /// Header parsing never surfaces an I/O error: a short read is a torn tail,
    /// not a failure, so this returns `Option` rather than `io::Result`. The
    /// payload I/O in [`WalEntry::apply`] is what can genuinely fail.
    pub(super) fn read(reader: &mut BufReader<File>, pos: u64) -> Option<Self> {
        let mut marker = [0u8; 1];
        if reader.read_exact(&mut marker).is_err() {
            return None;
        }

        let mut id_bytes = [0u8; 8];
        if reader.read_exact(&mut id_bytes).is_err() {
            // Torn tail: crashed after the marker but before the full id.
            return None;
        }
        let id = u64::from_le_bytes(id_bytes);
        let pos_after_header = pos + 1 + 8;

        let (op, has_crc) = match marker[0] {
            LEGACY_STORE_MARKER => (WalOp::Store { id }, false),
            LEGACY_DELETE_MARKER => (WalOp::Delete { id }, false),
            CRC_STORE_MARKER => (WalOp::Store { id }, true),
            CRC_DELETE_MARKER => (WalOp::Delete { id }, true),
            _ => {
                // Unknown marker: the framing is no longer trustworthy. The CRC
                // never covers the marker/length framing bytes, so a flipped
                // marker — or a shrunk length that desynchronised the previous
                // record's framing — lands here mid-stream. Stop replay cleanly
                // (keeping every entry read so far) and surface the corruption
                // via the shared metric + a warning, mirroring the vector WAL's
                // `#898` policy. Returning `None` matches the torn-tail
                // contract: the caller stops rather than failing the open.
                crate::metrics::global_guardrails_metrics().record_wal_replay_corrupt_entry();
                tracing::warn!(
                    marker = marker[0],
                    "Unknown WAL payload marker during replay — stopping at corrupt entry"
                );
                return None;
            }
        };

        Some(Self {
            op,
            pos_after_header,
            has_crc,
        })
    }

    /// Applies this entry to the index, returning the new file position.
    ///
    /// Returns `Ok(None)` for a torn tail (a record truncated by a crash
    /// mid-append) so the caller stops replay cleanly; see the module-level
    /// torn-tail policy.
    ///
    /// `wal_end` is the logical end of the WAL being replayed; it bounds the
    /// declared payload length so a corrupt length field cannot drive an
    /// unbounded allocation (#897/#898).
    pub(super) fn apply(
        self,
        index: &mut FxHashMap<u64, u64>,
        reader: &mut BufReader<File>,
        wal_end: u64,
    ) -> io::Result<Option<u64>> {
        match self.op {
            WalOp::Store { id } => self.apply_store(id, index, reader, wal_end),
            WalOp::Delete { id } => Ok(self.apply_delete(id, index, reader)),
        }
    }

    fn apply_store(
        &self,
        id: u64,
        index: &mut FxHashMap<u64, u64>,
        reader: &mut BufReader<File>,
        wal_end: u64,
    ) -> io::Result<Option<u64>> {
        let len_offset = self.pos_after_header;
        let mut len_bytes = [0u8; 4];
        if reader.read_exact(&mut len_bytes).is_err() {
            // Torn tail: crashed after the id but before the full length field.
            return Ok(None);
        }
        let payload_len = u64::from(u32::from_le_bytes(len_bytes));

        // OOM guard (#897/#898): the payload (plus the 4-byte CRC for v2
        // records) cannot extend past the WAL end. A length running past EOF is
        // a torn tail (crashed before the payload landed) — stop cleanly rather
        // than failing the open, and never allocate the oversized buffer.
        let payload_start = self.pos_after_header + 4;
        let crc_bytes = u64::from(self.has_crc) * 4;
        let max_payload = wal_end.saturating_sub(payload_start);
        if payload_len.saturating_add(crc_bytes) > max_payload {
            return Ok(None);
        }

        let end_pos = if self.has_crc {
            self.apply_store_with_crc(id, payload_len, index, reader, len_offset)?
        } else {
            let skip = i64::try_from(payload_len)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Payload too large"))?;
            reader.seek(SeekFrom::Current(skip))?;
            index.insert(id, len_offset);
            self.pos_after_header + 4 + payload_len
        };

        Ok(Some(end_pos))
    }

    fn apply_store_with_crc(
        &self,
        id: u64,
        payload_len: u64,
        index: &mut FxHashMap<u64, u64>,
        reader: &mut BufReader<File>,
        len_offset: u64,
    ) -> io::Result<u64> {
        let payload_usize = usize::try_from(payload_len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Payload too large"))?;
        let mut payload_buf = vec![0u8; payload_usize];
        reader.read_exact(&mut payload_buf)?;

        let mut crc_bytes = [0u8; 4];
        reader.read_exact(&mut crc_bytes)?;
        let stored_crc = u32::from_le_bytes(crc_bytes);
        let computed_crc = compute_store_crc(id, &payload_buf);

        if stored_crc == computed_crc {
            index.insert(id, len_offset);
        } else {
            tracing::warn!(
                id,
                "WAL CRC mismatch on store entry — skipping corrupted entry"
            );
        }

        Ok(self.pos_after_header + 4 + payload_len + 4)
    }

    fn apply_delete(
        &self,
        id: u64,
        index: &mut FxHashMap<u64, u64>,
        reader: &mut BufReader<File>,
    ) -> Option<u64> {
        if self.has_crc {
            let mut crc_bytes = [0u8; 4];
            if reader.read_exact(&mut crc_bytes).is_err() {
                // Torn tail: crashed after the id but before the CRC.
                return None;
            }
            let stored_crc = u32::from_le_bytes(crc_bytes);
            let computed_crc = compute_delete_crc(id);

            if stored_crc == computed_crc {
                index.remove(&id);
            } else {
                tracing::warn!(
                    id,
                    "WAL CRC mismatch on delete entry — skipping corrupted entry"
                );
            }

            Some(self.pos_after_header + 4)
        } else {
            index.remove(&id);
            Some(self.pos_after_header)
        }
    }
}
