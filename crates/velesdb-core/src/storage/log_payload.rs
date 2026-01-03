//! Log-structured payload storage.
//!
//! Stores payloads in an append-only log file with an in-memory index.

use super::traits::PayloadStorage;

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Log-structured payload storage.
///
/// Stores payloads in an append-only log file with an in-memory index.
#[allow(clippy::module_name_repetitions)]
pub struct LogPayloadStorage {
    _path: PathBuf,
    index: RwLock<FxHashMap<u64, u64>>, // ID -> Offset of length
    wal: RwLock<io::BufWriter<File>>,
    reader: RwLock<File>, // Independent file handle for reading, protected for seeking
}

impl LogPayloadStorage {
    /// Creates a new `LogPayloadStorage`.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;
        let log_path = path.join("payloads.log");

        // Open for writing (append)
        // create(true) implies write(true) if not append(true), but with append it works.
        // The warning likely points to redundant flags.
        let writer_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let wal = io::BufWriter::new(writer_file);

        // Open for reading
        let reader = File::open(&log_path)?;

        // Replay log to build index
        let mut index = FxHashMap::default();
        let len = reader.metadata()?.len();
        let mut pos = 0;
        let mut reader_buf = BufReader::new(&reader);

        while pos < len {
            // Read marker (1 byte)
            let mut marker = [0u8; 1];
            if reader_buf.read_exact(&mut marker).is_err() {
                break; // End of file
            }
            pos += 1;

            // Read ID (8 bytes)
            let mut id_bytes = [0u8; 8];
            reader_buf.read_exact(&mut id_bytes)?;
            let id = u64::from_le_bytes(id_bytes);
            pos += 8;

            if marker[0] == 1 {
                // Store
                // Record offset where LEN starts
                let len_offset = pos;

                // Read Len (4 bytes)
                let mut len_bytes = [0u8; 4];
                reader_buf.read_exact(&mut len_bytes)?;
                let payload_len = u64::from(u32::from_le_bytes(len_bytes));
                pos += 4;

                index.insert(id, len_offset);

                // Skip payload
                // Ensure payload_len fits in i64 for seek
                let skip = i64::try_from(payload_len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Payload too large"))?;
                reader_buf.seek(SeekFrom::Current(skip))?;
                pos += payload_len;
            } else if marker[0] == 2 {
                // Delete
                index.remove(&id);
            } else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Unknown marker"));
            }
        }

        // Re-open reader for random access
        let reader = File::open(&log_path)?;

        Ok(Self {
            _path: path,
            index: RwLock::new(index),
            wal: RwLock::new(wal),
            reader: RwLock::new(reader),
        })
    }
}

impl PayloadStorage for LogPayloadStorage {
    fn store(&mut self, id: u64, payload: &serde_json::Value) -> io::Result<()> {
        let payload_bytes = serde_json::to_vec(payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut wal = self.wal.write();
        let mut index = self.index.write();

        // Let's force flush to get accurate position or track it manually.
        wal.flush()?;
        let pos = wal.get_ref().metadata()?.len();

        // Op: Store (1) | ID | Len | Data
        // Pos points to start of record (Marker)
        // We want index to point to Len (Marker(1) + ID(8) = +9 bytes)

        wal.write_all(&[1u8])?;
        wal.write_all(&id.to_le_bytes())?;
        let len_u32 = u32::try_from(payload_bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Payload too large"))?;
        wal.write_all(&len_u32.to_le_bytes())?;
        wal.write_all(&payload_bytes)?;

        // Flush to ensure reader sees it
        wal.flush()?;

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

        wal.write_all(&[2u8])?;
        wal.write_all(&id.to_le_bytes())?;

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
