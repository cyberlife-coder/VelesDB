//! Log-structured payload storage with snapshot support.
//!
//! Stores payloads in an append-only log file with an in-memory index.
//! Supports periodic snapshots for fast cold-start recovery.
//!
//! # Snapshot System (P0 Optimization)
//!
//! Without snapshots, cold start requires replaying the entire WAL (O(N)).
//! With snapshots, we load the index directly and only replay the delta.
//!
//! ## Files
//!
//! - `payloads.log` - Append-only WAL (Write-Ahead Log)
//! - `payloads.snapshot` - Binary snapshot of the index
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

use super::traits::PayloadStorage;

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Snapshot file magic bytes.
pub(crate) const SNAPSHOT_MAGIC: &[u8; 4] = b"VSNP";

/// Current snapshot format version.
pub(crate) const SNAPSHOT_VERSION: u8 = 1;

/// Default threshold for automatic snapshot creation (10 MB of WAL since last snapshot).
const DEFAULT_SNAPSHOT_THRESHOLD: u64 = 10 * 1024 * 1024;

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

/// Log-structured payload storage with snapshot support.
///
/// Stores payloads in an append-only log file with an in-memory index.
/// Supports periodic snapshots for O(1) cold-start recovery instead of O(N) WAL replay.
#[allow(clippy::module_name_repetitions)]
pub struct LogPayloadStorage {
    /// Directory path for storage files
    path: PathBuf,
    /// In-memory index: ID -> Offset of length field in WAL
    index: RwLock<FxHashMap<u64, u64>>,
    /// Write-Ahead Log writer (append-only)
    wal: RwLock<io::BufWriter<File>>,
    /// Independent file handle for reading, protected for seeking
    reader: RwLock<File>,
    /// D-06 fix: WAL position at last snapshot (0 = no snapshot). AtomicU64 avoids RwLock overhead.
    last_snapshot_wal_pos: std::sync::atomic::AtomicU64,
    /// D-05 fix: in-memory WAL write position to avoid flush+metadata() per store.
    wal_write_pos: std::sync::atomic::AtomicU64,
}

impl LogPayloadStorage {
    /// Creates a new `LogPayloadStorage` or opens an existing one.
    ///
    /// If a snapshot file exists and is valid, loads from snapshot and replays
    /// only the WAL delta for fast startup. Otherwise, falls back to full WAL replay.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;
        let log_path = path.join("payloads.log");
        let snapshot_path = path.join("payloads.snapshot");

        // Open WAL for writing (append)
        let writer_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let wal = io::BufWriter::new(writer_file);

        // Open reader for random access
        // Create empty file if it doesn't exist
        if !log_path.exists() {
            File::create(&log_path)?;
        }
        let reader = File::open(&log_path)?;
        let wal_len = reader.metadata()?.len();

        // Try to load from snapshot, fall back to full WAL replay
        let (index, last_snapshot_wal_pos) =
            if let Ok((snapshot_index, snapshot_wal_pos)) = Self::load_snapshot(&snapshot_path) {
                // Replay WAL delta (entries after snapshot)
                let index =
                    Self::replay_wal_from(&log_path, snapshot_index, snapshot_wal_pos, wal_len)?;
                (index, snapshot_wal_pos)
            } else {
                // No valid snapshot, full WAL replay
                let index = Self::replay_wal_from(&log_path, FxHashMap::default(), 0, wal_len)?;
                (index, 0)
            };

        Ok(Self {
            path,
            index: RwLock::new(index),
            wal: RwLock::new(wal),
            reader: RwLock::new(reader),
            last_snapshot_wal_pos: std::sync::atomic::AtomicU64::new(last_snapshot_wal_pos),
            wal_write_pos: std::sync::atomic::AtomicU64::new(wal_len),
        })
    }

    /// Replays WAL entries from `start_pos` to `end_pos`, updating the index.
    ///
    /// D-04 fix: verifies CRC32 per entry. Stops replay on CRC mismatch
    /// (truncated/corrupted tail), preserving all valid entries before it.
    fn replay_wal_from(
        log_path: &Path,
        mut index: FxHashMap<u64, u64>,
        start_pos: u64,
        end_pos: u64,
    ) -> io::Result<FxHashMap<u64, u64>> {
        if start_pos >= end_pos {
            return Ok(index);
        }

        let file = File::open(log_path)?;
        let mut reader_buf = BufReader::new(file);
        reader_buf.seek(SeekFrom::Start(start_pos))?;

        let mut pos = start_pos;

        while pos < end_pos {
            // Read marker (1 byte)
            let mut marker = [0u8; 1];
            if reader_buf.read_exact(&mut marker).is_err() {
                break;
            }
            pos += 1;

            // Read ID (8 bytes)
            let mut id_bytes = [0u8; 8];
            if reader_buf.read_exact(&mut id_bytes).is_err() {
                break;
            }
            let id = u64::from_le_bytes(id_bytes);
            pos += 8;

            if marker[0] == 1 {
                // Store operation: Op(1) | ID(8) | Len(4) | Data(N) | CRC(4)
                let len_offset = pos;

                // Read Len (4 bytes)
                let mut len_bytes = [0u8; 4];
                reader_buf.read_exact(&mut len_bytes)?;
                let payload_len = u64::from(u32::from_le_bytes(len_bytes));
                pos += 4;

                // Read payload data for CRC verification
                let payload_usize = usize::try_from(payload_len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Payload too large"))?;
                let mut payload_data = vec![0u8; payload_usize];
                reader_buf.read_exact(&mut payload_data)?;
                pos += payload_len;

                // D-04: Read and verify CRC
                let mut crc_bytes = [0u8; 4];
                if reader_buf.read_exact(&mut crc_bytes).is_err() {
                    // Truncated CRC — treat as incomplete entry, stop replay
                    tracing::warn!("WAL replay: truncated CRC at pos {pos}, stopping");
                    break;
                }
                let stored_crc = u32::from_le_bytes(crc_bytes);
                pos += 4;

                // Recompute CRC over Op + ID + Len + Data
                let mut crc_buf = Vec::with_capacity(1 + 8 + 4 + payload_usize);
                crc_buf.push(marker[0]);
                crc_buf.extend_from_slice(&id_bytes);
                crc_buf.extend_from_slice(&len_bytes);
                crc_buf.extend_from_slice(&payload_data);
                let computed_crc = crc32_hash(&crc_buf);

                if stored_crc != computed_crc {
                    tracing::warn!(
                        "WAL replay: CRC mismatch at pos {}, expected {:#010x}, got {:#010x}. Stopping replay.",
                        pos - 4, computed_crc, stored_crc
                    );
                    break;
                }

                index.insert(id, len_offset);
            } else if marker[0] == 2 {
                // Delete operation: Op(1) | ID(8) | CRC(4)
                // D-04: Read and verify CRC
                let mut crc_bytes = [0u8; 4];
                if reader_buf.read_exact(&mut crc_bytes).is_err() {
                    tracing::warn!("WAL replay: truncated delete CRC at pos {pos}, stopping");
                    break;
                }
                let stored_crc = u32::from_le_bytes(crc_bytes);
                pos += 4;

                let mut crc_buf = [0u8; 9];
                crc_buf[0] = marker[0];
                crc_buf[1..9].copy_from_slice(&id_bytes);
                let computed_crc = crc32_hash(&crc_buf);

                if stored_crc != computed_crc {
                    tracing::warn!(
                        "WAL replay: delete CRC mismatch at pos {}, stopping.",
                        pos - 4
                    );
                    break;
                }

                index.remove(&id);
            } else {
                // Unknown marker — possibly legacy WAL without CRC.
                // Try to handle gracefully by stopping replay.
                tracing::warn!(
                    "WAL replay: unknown marker {} at pos {}, stopping.",
                    marker[0],
                    pos - 9
                );
                break;
            }
        }

        Ok(index)
    }

    /// Loads index from snapshot file.
    ///
    /// Returns (index, `wal_position`) if successful.
    fn load_snapshot(snapshot_path: &Path) -> io::Result<(FxHashMap<u64, u64>, u64)> {
        if !snapshot_path.exists() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "No snapshot"));
        }

        let data = std::fs::read(snapshot_path)?;

        // Validate minimum size: magic(4) + version(1) + wal_pos(8) + count(8) + crc(4) = 25
        if data.len() < 25 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Snapshot too small",
            ));
        }

        // Validate magic
        if &data[0..4] != SNAPSHOT_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid magic"));
        }

        // Validate version
        if data[4] != SNAPSHOT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported version",
            ));
        }

        // Read WAL position
        let wal_pos = u64::from_le_bytes(
            data[5..13]
                .try_into()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid WAL position"))?,
        );

        // Read entry count
        let entry_count_u64 = u64::from_le_bytes(
            data[13..21]
                .try_into()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid entry count"))?,
        );

        // P1 Audit: Validate entry_count BEFORE conversion to prevent DoS via huge values
        // Max reasonable entry count: data.len() / 16 (minimum entry size)
        // This check prevents both overflow and OOM attacks
        let max_possible_entries = data.len().saturating_sub(25) / 16; // header(21) + crc(4) = 25
        if entry_count_u64 > max_possible_entries as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Entry count exceeds data size",
            ));
        }

        #[allow(clippy::cast_possible_truncation)] // Validated above
        let entry_count = entry_count_u64 as usize;

        // Validate size: header(21) + entries(entry_count * 16) + crc(4)
        // Safe: entry_count is validated to not cause overflow
        let expected_size = 21 + entry_count * 16 + 4;
        if data.len() != expected_size {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Size mismatch"));
        }

        // Validate CRC
        let stored_crc = u32::from_le_bytes(
            data[data.len() - 4..]
                .try_into()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid CRC"))?,
        );
        let computed_crc = crc32_hash(&data[..data.len() - 4]);
        if stored_crc != computed_crc {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "CRC mismatch"));
        }

        // Read entries
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
            let wal_offset =
                u64::from_le_bytes(data[offset + 8..offset + 16].try_into().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "Invalid entry offset")
                })?);
            index.insert(id, wal_offset);
        }

        Ok((index, wal_pos))
    }

    /// Creates a snapshot of the current index state.
    ///
    /// The snapshot captures:
    /// - Current WAL position
    /// - All index entries (ID -> offset mappings)
    /// - CRC32 checksum for integrity
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn create_snapshot(&mut self) -> io::Result<()> {
        // Flush WAL first to ensure all writes are on disk
        self.wal.write().flush()?;

        let snapshot_path = self.path.join("payloads.snapshot");
        let index = self.index.read();

        // Get current WAL position
        let wal_pos = self.wal.write().get_ref().metadata()?.len();

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
        for (&id, &offset) in index.iter() {
            buf.extend_from_slice(&id.to_le_bytes());
            buf.extend_from_slice(&offset.to_le_bytes());
        }

        // Compute and append CRC
        let crc = crc32_hash(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        // Write atomically via temp file + rename
        let temp_path = self.path.join("payloads.snapshot.tmp");
        std::fs::write(&temp_path, &buf)?;
        std::fs::rename(&temp_path, &snapshot_path)?;

        // Update last snapshot position
        self.last_snapshot_wal_pos
            .store(wal_pos, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Returns whether a new snapshot should be created.
    ///
    /// Heuristic: Returns true if WAL has grown by more than `DEFAULT_SNAPSHOT_THRESHOLD`
    /// bytes since the last snapshot.
    #[must_use]
    pub fn should_create_snapshot(&self) -> bool {
        let last_pos = self
            .last_snapshot_wal_pos
            .load(std::sync::atomic::Ordering::Relaxed);
        // D-06 fix: use in-memory position instead of wal.write().get_ref().metadata()
        let current_pos = self
            .wal_write_pos
            .load(std::sync::atomic::Ordering::Relaxed);

        current_pos.saturating_sub(last_pos) >= DEFAULT_SNAPSHOT_THRESHOLD
    }
}

impl PayloadStorage for LogPayloadStorage {
    fn store(&mut self, id: u64, payload: &serde_json::Value) -> io::Result<()> {
        let payload_bytes = serde_json::to_vec(payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut wal = self.wal.write();
        let mut index = self.index.write();

        // D-05 fix: use in-memory position tracker instead of flush+metadata().
        let pos = self
            .wal_write_pos
            .load(std::sync::atomic::Ordering::Relaxed);

        // D-04 fix: compute CRC32 over the entire entry (Op + ID + Len + Data).
        let len_u32 = u32::try_from(payload_bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Payload too large"))?;
        let mut crc_buf = Vec::with_capacity(1 + 8 + 4 + payload_bytes.len());
        crc_buf.push(1u8);
        crc_buf.extend_from_slice(&id.to_le_bytes());
        crc_buf.extend_from_slice(&len_u32.to_le_bytes());
        crc_buf.extend_from_slice(&payload_bytes);
        let crc = crc32_hash(&crc_buf);

        // Write: Op(1) | ID(8) | Len(4) | Data(N) | CRC(4)
        wal.write_all(&crc_buf)?;
        wal.write_all(&crc.to_le_bytes())?;
        wal.flush()?;

        // Entry size: 1 + 8 + 4 + N + 4 = 17 + N
        let entry_size = 1 + 8 + 4 + payload_bytes.len() as u64 + 4;
        self.wal_write_pos
            .store(pos + entry_size, std::sync::atomic::Ordering::Relaxed);

        // Index points to Len field offset (Marker(1) + ID(8) = +9 bytes from entry start)
        index.insert(id, pos + 9);

        Ok(())
    }

    fn retrieve(&self, id: u64) -> io::Result<Option<serde_json::Value>> {
        let index = self.index.read();
        let Some(&offset) = index.get(&id) else {
            return Ok(None);
        };
        drop(index);

        let mut reader = self.reader.write(); // Need write lock to seek
        reader.seek(SeekFrom::Start(offset))?;

        let mut len_bytes = [0u8; 4];
        reader.read_exact(&mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        let mut payload_bytes = vec![0u8; len];
        reader.read_exact(&mut payload_bytes)?;

        let payload = serde_json::from_slice(&payload_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        Ok(Some(payload))
    }

    fn delete(&mut self, id: u64) -> io::Result<()> {
        let mut wal = self.wal.write();
        let mut index = self.index.write();

        // D-04 fix: compute CRC32 over Op + ID
        let mut crc_buf = [0u8; 9];
        crc_buf[0] = 2u8;
        crc_buf[1..9].copy_from_slice(&id.to_le_bytes());
        let crc = crc32_hash(&crc_buf);

        // Write: Op(1) | ID(8) | CRC(4)
        wal.write_all(&crc_buf)?;
        wal.write_all(&crc.to_le_bytes())?;

        // D-05 fix: update in-memory position (entry size: 1 + 8 + 4 = 13)
        self.wal_write_pos
            .fetch_add(13, std::sync::atomic::Ordering::Relaxed);

        index.remove(&id);

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.wal.write().flush()
    }

    fn ids(&self) -> Vec<u64> {
        self.index.read().keys().copied().collect()
    }
}
