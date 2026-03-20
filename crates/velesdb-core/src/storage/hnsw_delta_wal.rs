//! HNSW delta WAL for incremental graph mutation logging.
//!
//! Records edge additions, edge removals, and entry point changes as
//! compact CRC32-framed entries. On recovery, replaying the delta WAL
//! is O(delta) instead of rebuilding the entire graph O(N*M).
//!
//! ## Wire Format (CRC32-framed)
//!
//! ```text
//! AddEdge:    [op=1: 1B] [from: 4B LE] [to: 4B LE] [layer: 1B] [crc32: 4B LE]  = 14 bytes
//! RemoveEdge: [op=2: 1B] [from: 4B LE] [to: 4B LE] [layer: 1B] [crc32: 4B LE]  = 14 bytes
//! SetEntry:   [op=3: 1B] [node: 4B LE] [max_layer: 1B] [crc32: 4B LE]           = 10 bytes
//! ```
//!
//! Reference: P-HNSW (MDPI 2024)

use super::snapshot::crc32_hash;

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

/// Size of the CRC32 trailer on every WAL frame.
const CRC_SIZE: usize = 4;

/// Payload size for `AddEdge` / `RemoveEdge` (op + from + to + layer).
const EDGE_PAYLOAD_SIZE: usize = 1 + 4 + 4 + 1; // 10 bytes

/// Payload size for `SetEntry` (op + node + max_layer).
const ENTRY_PAYLOAD_SIZE: usize = 1 + 4 + 1; // 6 bytes

/// Operation codes for HNSW delta WAL entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DeltaOp {
    /// Directed edge addition.
    AddEdge = 1,
    /// Directed edge removal.
    RemoveEdge = 2,
    /// Entry point / max-layer update.
    SetEntry = 3,
}

/// A single HNSW graph mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HnswDelta {
    /// Add a directed edge from `from` to `to` at `layer`.
    AddEdge {
        /// Source node ID.
        from: u32,
        /// Destination node ID.
        to: u32,
        /// HNSW layer.
        layer: u8,
    },
    /// Remove a directed edge from `from` to `to` at `layer`.
    RemoveEdge {
        /// Source node ID.
        from: u32,
        /// Destination node ID.
        to: u32,
        /// HNSW layer.
        layer: u8,
    },
    /// Set the graph entry point and max layer.
    SetEntry {
        /// Entry point node ID.
        node: u32,
        /// Maximum layer of the entry point.
        max_layer: u8,
    },
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

/// Serializes a delta into the given buffer (payload only, no CRC).
fn serialize_delta(delta: &HnswDelta, buf: &mut Vec<u8>) {
    match *delta {
        HnswDelta::AddEdge { from, to, layer } => {
            buf.push(DeltaOp::AddEdge as u8);
            buf.extend_from_slice(&from.to_le_bytes());
            buf.extend_from_slice(&to.to_le_bytes());
            buf.push(layer);
        }
        HnswDelta::RemoveEdge { from, to, layer } => {
            buf.push(DeltaOp::RemoveEdge as u8);
            buf.extend_from_slice(&from.to_le_bytes());
            buf.extend_from_slice(&to.to_le_bytes());
            buf.push(layer);
        }
        HnswDelta::SetEntry { node, max_layer } => {
            buf.push(DeltaOp::SetEntry as u8);
            buf.extend_from_slice(&node.to_le_bytes());
            buf.push(max_layer);
        }
    }
}

/// Parses a delta from a raw payload buffer (no CRC).
///
/// Returns `None` if the op byte is unrecognized.
fn deserialize_delta(payload: &[u8]) -> Option<HnswDelta> {
    let op = *payload.first()?;
    match op {
        1 if payload.len() == EDGE_PAYLOAD_SIZE => {
            let from = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            let to = u32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]);
            let layer = payload[9];
            Some(HnswDelta::AddEdge { from, to, layer })
        }
        2 if payload.len() == EDGE_PAYLOAD_SIZE => {
            let from = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            let to = u32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]);
            let layer = payload[9];
            Some(HnswDelta::RemoveEdge { from, to, layer })
        }
        3 if payload.len() == ENTRY_PAYLOAD_SIZE => {
            let node = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            let max_layer = payload[5];
            Some(HnswDelta::SetEntry { node, max_layer })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Writes HNSW delta entries to a WAL file.
///
/// Each entry is serialized as a compact binary frame with a CRC32 trailer
/// for integrity verification on recovery.
pub struct HnswDeltaWriter {
    writer: BufWriter<File>,
    entry_count: u64,
}

impl HnswDeltaWriter {
    /// Opens or creates a delta WAL file at the given path.
    ///
    /// New entries are appended to the end of the file.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file cannot be opened or created.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            entry_count: 0,
        })
    }

    /// Appends a delta entry to the WAL.
    ///
    /// The entry is serialized and a CRC32 trailer is appended for
    /// integrity verification during recovery.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the write fails.
    pub fn append(&mut self, delta: &HnswDelta) -> io::Result<()> {
        let mut buf = Vec::with_capacity(EDGE_PAYLOAD_SIZE + CRC_SIZE);
        serialize_delta(delta, &mut buf);

        let crc = crc32_hash(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        self.writer.write_all(&buf)?;
        self.entry_count += 1;
        Ok(())
    }

    /// Flushes buffered writes and syncs to disk.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the flush or sync fails.
    pub fn sync(&mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_all()
    }

    /// Returns the number of entries written since this writer was opened.
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.entry_count
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Reads HNSW delta entries from a WAL file.
///
/// On recovery, entries are read sequentially until EOF or the first
/// corrupted frame (indicating a crash during write). All valid entries
/// before the corruption boundary are returned.
pub struct HnswDeltaReader {
    reader: BufReader<File>,
}

impl HnswDeltaReader {
    /// Opens an existing delta WAL file for reading.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file cannot be opened.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            reader: BufReader::new(file),
        })
    }

    /// Reads all valid entries from the WAL.
    ///
    /// Stops at the first corrupted or incomplete entry (crash boundary).
    /// All entries that passed CRC validation before that point are returned.
    ///
    /// # Errors
    ///
    /// Returns an I/O error only for unexpected system-level failures (not
    /// for EOF or CRC mismatches, which are normal crash-recovery signals).
    pub fn read_all(&mut self) -> io::Result<Vec<HnswDelta>> {
        let mut entries = Vec::new();
        loop {
            match self.read_one() {
                Ok(Some(delta)) => entries.push(delta),
                Ok(None) => break,
                // Truncated write or corrupted frame — treat as crash boundary.
                Err(e)
                    if e.kind() == io::ErrorKind::UnexpectedEof
                        || e.kind() == io::ErrorKind::InvalidData =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(entries)
    }
}

/// Reads a single framed entry from the WAL.
///
/// Returns `Ok(None)` on clean EOF at an entry boundary.
/// Returns `Err(UnexpectedEof)` for truncated entries (crash mid-write).
/// Returns `Err(InvalidData)` for CRC mismatches.
impl HnswDeltaReader {
    fn read_one(&mut self) -> io::Result<Option<HnswDelta>> {
        // Read op byte — EOF here is a clean boundary.
        let mut op_buf = [0u8; 1];
        match self.reader.read_exact(&mut op_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }

        let payload_size = payload_size_for_op(op_buf[0])?;
        let mut payload = vec![0u8; payload_size];
        payload[0] = op_buf[0];
        self.reader.read_exact(&mut payload[1..])?;

        // Read and verify CRC.
        let mut crc_buf = [0u8; CRC_SIZE];
        self.reader.read_exact(&mut crc_buf)?;
        let stored_crc = u32::from_le_bytes(crc_buf);
        let computed_crc = crc32_hash(&payload);

        if stored_crc != computed_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CRC32 mismatch in HNSW delta WAL entry",
            ));
        }

        deserialize_delta(&payload).map_or_else(
            || {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unrecognized HNSW delta op",
                ))
            },
            |d| Ok(Some(d)),
        )
    }
}

/// Returns the total payload size (including op byte) for a given op code.
fn payload_size_for_op(op: u8) -> io::Result<usize> {
    match op {
        1 | 2 => Ok(EDGE_PAYLOAD_SIZE),
        3 => Ok(ENTRY_PAYLOAD_SIZE),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown HNSW delta WAL op code",
        )),
    }
}
