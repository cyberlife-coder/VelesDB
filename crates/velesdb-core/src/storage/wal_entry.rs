//! WAL entry domain type — separates parsing from application.
//!
//! Extracted from `log_payload.rs` to reduce NLOC.

use super::log_payload::{
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
    /// Reads one WAL entry from the reader. Returns `None` on EOF.
    pub(super) fn read(reader: &mut BufReader<File>, pos: u64) -> io::Result<Option<Self>> {
        let mut marker = [0u8; 1];
        if reader.read_exact(&mut marker).is_err() {
            return Ok(None);
        }

        let mut id_bytes = [0u8; 8];
        reader.read_exact(&mut id_bytes)?;
        let id = u64::from_le_bytes(id_bytes);
        let pos_after_header = pos + 1 + 8;

        let (op, has_crc) = match marker[0] {
            LEGACY_STORE_MARKER => (WalOp::Store { id }, false),
            LEGACY_DELETE_MARKER => (WalOp::Delete { id }, false),
            CRC_STORE_MARKER => (WalOp::Store { id }, true),
            CRC_DELETE_MARKER => (WalOp::Delete { id }, true),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unknown WAL marker",
                ))
            }
        };

        Ok(Some(Self {
            op,
            pos_after_header,
            has_crc,
        }))
    }

    /// Applies this entry to the index, returning the new file position.
    pub(super) fn apply(
        self,
        index: &mut FxHashMap<u64, u64>,
        reader: &mut BufReader<File>,
    ) -> io::Result<u64> {
        match self.op {
            WalOp::Store { id } => self.apply_store(id, index, reader),
            WalOp::Delete { id } => self.apply_delete(id, index, reader),
        }
    }

    fn apply_store(
        &self,
        id: u64,
        index: &mut FxHashMap<u64, u64>,
        reader: &mut BufReader<File>,
    ) -> io::Result<u64> {
        let len_offset = self.pos_after_header;
        let mut len_bytes = [0u8; 4];
        reader.read_exact(&mut len_bytes)?;
        let payload_len = u64::from(u32::from_le_bytes(len_bytes));

        let end_pos = if self.has_crc {
            self.apply_store_with_crc(id, payload_len, index, reader, len_offset)?
        } else {
            let skip = i64::try_from(payload_len)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Payload too large"))?;
            reader.seek(SeekFrom::Current(skip))?;
            index.insert(id, len_offset);
            self.pos_after_header + 4 + payload_len
        };

        Ok(end_pos)
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
    ) -> io::Result<u64> {
        if self.has_crc {
            let mut crc_bytes = [0u8; 4];
            reader.read_exact(&mut crc_bytes)?;
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

            Ok(self.pos_after_header + 4)
        } else {
            index.remove(&id);
            Ok(self.pos_after_header)
        }
    }
}
