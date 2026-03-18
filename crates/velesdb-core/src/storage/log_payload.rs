//! Log-structured payload storage with snapshot support.
//!
//! Stores payloads in an append-only log file with an in-memory index.
//! Supports periodic snapshots for fast cold-start recovery.
//!
//! Snapshot format and I/O are handled by the [`super::snapshot`] module.

use super::snapshot;
use super::traits::PayloadStorage;

// Re-export snapshot items for backward compatibility with existing imports
#[allow(unused_imports)] // SNAPSHOT_MAGIC/VERSION used only in test modules
pub(crate) use snapshot::{crc32_hash, SNAPSHOT_MAGIC, SNAPSHOT_VERSION};

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Controls how payload WAL writes are synced to disk.
///
/// - `Fsync` (default): `flush()` + `sync_all()` — full durability, safe against power loss.
/// - `FlushOnly`: `flush()` only — data reaches OS kernel but may be lost on power failure.
/// - `None`: No sync — maximum throughput for bulk imports where data can be re-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DurabilityMode {
    /// Full durability: flush buffer + fsync to disk.
    #[default]
    Fsync,
    /// Flush buffer to OS only (no fsync). Faster but not power-loss safe.
    FlushOnly,
    /// No sync at all. Maximum throughput for bulk imports.
    None,
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
    /// WAL position at last snapshot (0 = no snapshot)
    last_snapshot_wal_pos: RwLock<u64>,
    /// Durability mode for WAL writes
    durability: DurabilityMode,
    /// Tracked WAL write position (avoids flush+metadata syscall for `DurabilityMode::None`)
    write_offset: RwLock<u64>,
}

// ---------------------------------------------------------------------------
// WAL entry domain type — separates parsing from application
// ---------------------------------------------------------------------------

/// A parsed WAL entry with its file position context.
struct WalEntry {
    op: WalOp,
    /// File position after the marker + ID header (start of payload length for Store).
    pos_after_header: u64,
}

/// The two WAL operations: store (upsert) or delete.
enum WalOp {
    Store { id: u64 },
    Delete { id: u64 },
}

impl WalEntry {
    /// Reads one WAL entry from the reader. Returns `None` on EOF.
    fn read(reader: &mut BufReader<File>, pos: u64) -> io::Result<Option<Self>> {
        let mut marker = [0u8; 1];
        if reader.read_exact(&mut marker).is_err() {
            return Ok(None); // EOF
        }

        let mut id_bytes = [0u8; 8];
        reader.read_exact(&mut id_bytes)?;
        let id = u64::from_le_bytes(id_bytes);
        let pos_after_header = pos + 1 + 8;

        let op = match marker[0] {
            1 => WalOp::Store { id },
            2 => WalOp::Delete { id },
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
        }))
    }

    /// Applies this entry to the index, returning the new file position.
    fn apply(
        self,
        index: &mut FxHashMap<u64, u64>,
        reader: &mut BufReader<File>,
    ) -> io::Result<u64> {
        match self.op {
            WalOp::Store { id } => {
                let len_offset = self.pos_after_header;
                let mut len_bytes = [0u8; 4];
                reader.read_exact(&mut len_bytes)?;
                let payload_len = u64::from(u32::from_le_bytes(len_bytes));

                index.insert(id, len_offset);

                let skip = i64::try_from(payload_len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Payload too large"))?;
                reader.seek(SeekFrom::Current(skip))?;
                Ok(self.pos_after_header + 4 + payload_len)
            }
            WalOp::Delete { id } => {
                index.remove(&id);
                Ok(self.pos_after_header)
            }
        }
    }
}

impl LogPayloadStorage {
    /// Creates a new `LogPayloadStorage` with the default durability mode (`Fsync`).
    ///
    /// If a snapshot file exists and is valid, loads from snapshot and replays
    /// only the WAL delta for fast startup. Otherwise, falls back to full WAL replay.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::new_with_durability(path, DurabilityMode::default())
    }

    /// Creates a new `LogPayloadStorage` with the specified durability mode.
    ///
    /// See [`DurabilityMode`] for available modes and their trade-offs.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn new_with_durability<P: AsRef<Path>>(
        path: P,
        durability: DurabilityMode,
    ) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;
        let log_path = path.join("payloads.log");

        let wal = Self::open_wal_writer(&log_path)?;
        let (reader, wal_len) = Self::open_wal_reader(&log_path)?;
        let (index, last_snapshot_wal_pos) = Self::load_or_replay_index(&path, &log_path, wal_len)?;

        Ok(Self {
            path,
            index: RwLock::new(index),
            wal: RwLock::new(wal),
            reader: RwLock::new(reader),
            last_snapshot_wal_pos: RwLock::new(last_snapshot_wal_pos),
            durability,
            write_offset: RwLock::new(wal_len),
        })
    }

    /// Opens the WAL file for append-mode writing.
    fn open_wal_writer(log_path: &Path) -> io::Result<io::BufWriter<File>> {
        let writer_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        Ok(io::BufWriter::new(writer_file))
    }

    /// Opens the WAL file for random-access reading, creating it if absent.
    ///
    /// Returns the reader handle and the current WAL length in bytes.
    fn open_wal_reader(log_path: &Path) -> io::Result<(File, u64)> {
        if !log_path.exists() {
            File::create(log_path)?;
        }
        let reader = File::open(log_path)?;
        let wal_len = reader.metadata()?.len();
        Ok((reader, wal_len))
    }

    /// Loads the payload index, trying a snapshot first, falling back to full WAL replay.
    ///
    /// Returns `(index, last_snapshot_wal_position)`.
    fn load_or_replay_index(
        dir: &Path,
        log_path: &Path,
        wal_len: u64,
    ) -> io::Result<(FxHashMap<u64, u64>, u64)> {
        let snapshot_path = dir.join("payloads.snapshot");
        if let Ok((snapshot_index, snapshot_wal_pos)) = snapshot::load_snapshot(&snapshot_path) {
            let index = Self::replay_wal_from(log_path, snapshot_index, snapshot_wal_pos, wal_len)?;
            Ok((index, snapshot_wal_pos))
        } else {
            let index = Self::replay_wal_from(log_path, FxHashMap::default(), 0, wal_len)?;
            Ok((index, 0))
        }
    }

    /// Applies the configured durability mode to a WAL writer.
    fn sync_wal(wal: &mut io::BufWriter<File>, mode: DurabilityMode) -> io::Result<()> {
        match mode {
            DurabilityMode::Fsync => {
                wal.flush()?;
                wal.get_ref().sync_all()?;
            }
            DurabilityMode::FlushOnly => {
                wal.flush()?;
            }
            DurabilityMode::None => {}
        }
        Ok(())
    }

    /// Syncs the WAL according to durability mode, resyncing `write_offset`
    /// with the actual file length on failure to prevent desync on subsequent
    /// writes.
    ///
    /// RF-2: Shared by `store` and `delete` to eliminate duplicated
    /// sync-and-resync-offset error handling.
    fn sync_wal_or_resync(
        wal: &mut io::BufWriter<File>,
        mode: DurabilityMode,
        offset: &mut u64,
    ) -> io::Result<()> {
        if let Err(e) = Self::sync_wal(wal, mode) {
            if let Ok(meta) = wal.get_ref().metadata() {
                *offset = meta.len();
            }
            return Err(e);
        }
        Ok(())
    }

    /// Replays WAL entries from `start_pos` to `end_pos`, updating the index.
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
            let Some(entry) = WalEntry::read(&mut reader_buf, pos)? else {
                break;
            };
            pos = entry.apply(&mut index, &mut reader_buf)?;
        }

        Ok(index)
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
        // Flush WAL before snapshotting to ensure data is on disk for the reader
        {
            let mut wal = self.wal.write();
            wal.flush()?;
            wal.get_ref().sync_all()?;
        }

        let index = self.index.read();
        let wal_pos = *self.write_offset.read();

        snapshot::create_snapshot_file(&self.path, &index, wal_pos)?;

        *self.last_snapshot_wal_pos.write() = wal_pos;

        Ok(())
    }

    /// Returns whether a new snapshot should be created.
    ///
    /// Heuristic: Returns true if WAL has grown by more than the default threshold
    /// bytes since the last snapshot.
    #[must_use]
    pub fn should_create_snapshot(&self) -> bool {
        snapshot::should_create_snapshot(
            *self.last_snapshot_wal_pos.read(),
            *self.write_offset.read(),
        )
    }
}

impl PayloadStorage for LogPayloadStorage {
    fn store(&mut self, id: u64, payload: &serde_json::Value) -> io::Result<()> {
        let payload_bytes = serde_json::to_vec(payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let len_u32 = u32::try_from(payload_bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Payload too large"))?;

        let mut wal = self.wal.write();
        let mut index = self.index.write();
        let mut offset = self.write_offset.write();

        let record_start = *offset;

        // H-3: Build complete record in one buffer to minimize partial-write window.
        // Op: Store (1) | ID (8) | Len (4) | Data (N)
        let mut record = Vec::with_capacity(1 + 8 + 4 + payload_bytes.len());
        record.push(1u8);
        record.extend_from_slice(&id.to_le_bytes());
        record.extend_from_slice(&len_u32.to_le_bytes());
        record.extend_from_slice(&payload_bytes);
        wal.write_all(&record)?;

        // Sync WAL according to durability mode (resync offset on failure).
        Self::sync_wal_or_resync(&mut wal, self.durability, &mut offset)?;

        // Marker(1) + ID(8) = 9 bytes before the length field
        let bytes_written = 1 + 8 + 4 + u64::from(len_u32);
        *offset += bytes_written;
        index.insert(id, record_start + 9);

        Ok(())
    }

    fn retrieve(&self, id: u64) -> io::Result<Option<serde_json::Value>> {
        let index = self.index.read();
        let Some(&offset) = index.get(&id) else {
            return Ok(None);
        };
        drop(index);

        // H-2: Only flush when DurabilityMode::None is configured, because sync_wal()
        // already flushes the BufWriter after every write in Fsync and FlushOnly modes.
        // Skipping this avoids acquiring the WAL write lock on every read, which would
        // serialize all readers behind writers.
        if self.durability == DurabilityMode::None {
            self.wal.write().flush()?;
        }

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
        let mut offset = self.write_offset.write();

        // H-3: Build complete delete record in one buffer to minimize partial-write window.
        // Op: Delete (1) | ID (8)
        let mut record = [0u8; 1 + 8];
        record[0] = 2u8;
        record[1..9].copy_from_slice(&id.to_le_bytes());
        wal.write_all(&record)?;

        // Sync WAL according to durability mode (resync offset on failure).
        Self::sync_wal_or_resync(&mut wal, self.durability, &mut offset)?;

        *offset += 1 + 8; // Marker(1) + ID(8)
        index.remove(&id);

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut wal = self.wal.write();
        Self::sync_wal(&mut wal, self.durability)
    }

    fn ids(&self) -> Vec<u64> {
        self.index.read().keys().copied().collect()
    }
}
