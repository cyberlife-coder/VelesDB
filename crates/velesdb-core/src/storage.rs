//! Storage backends for persistent vector storage.
//!
//! This module contains memory-mapped file storage implementation for vectors
//! and log-structured storage for metadata payloads.

use memmap2::MmapMut;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Trait defining storage operations for vectors.
pub trait VectorStorage: Send + Sync {
    /// Stores a vector with the given ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    fn store(&mut self, id: u64, vector: &[f32]) -> io::Result<()>;

    /// Stores multiple vectors in a single batch operation.
    ///
    /// This is optimized for bulk imports:
    /// - Single WAL write for the entire batch
    /// - Contiguous memory writes
    /// - Single fsync at the end
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    fn store_batch(&mut self, vectors: &[(u64, &[f32])]) -> io::Result<usize>;

    /// Retrieves a vector by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the read operation fails.
    fn retrieve(&self, id: u64) -> io::Result<Option<Vec<f32>>>;

    /// Deletes a vector by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete operation fails.
    fn delete(&mut self, id: u64) -> io::Result<()>;

    /// Flushes pending writes to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush operation fails.
    fn flush(&mut self) -> io::Result<()>;

    /// Returns the number of vectors stored.
    fn len(&self) -> usize;

    /// Returns true if the storage is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Memory-mapped file storage for vectors.
///
/// Uses a combination of an index file (ID -> offset) and a data file (raw vectors).
/// Also implements a simple WAL for durability.
#[allow(clippy::module_name_repetitions)]
pub struct MmapStorage {
    /// Directory path for storage files
    path: PathBuf,
    /// Vector dimension
    dimension: usize,
    /// In-memory index of ID -> file offset
    index: RwLock<HashMap<u64, usize>>,
    /// Write-Ahead Log writer
    wal: RwLock<io::BufWriter<File>>,
    /// File handle for the data file (kept open for resizing)
    data_file: File,
    /// Memory mapped data file
    mmap: RwLock<MmapMut>,
    /// Next available offset in the data file
    next_offset: AtomicUsize,
}

impl MmapStorage {
    const INITIAL_SIZE: u64 = 64 * 1024; // 64KB initial size
    const MIN_GROWTH: u64 = 1024 * 1024; // Minimum 1MB growth

    /// Creates a new `MmapStorage` or opens an existing one.
    ///
    /// # Arguments
    ///
    /// * `path` - Directory to store data
    /// * `dimension` - Vector dimension
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn new<P: AsRef<Path>>(path: P, dimension: usize) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;

        // 1. Open/Create Data File
        let data_path = path.join("vectors.dat");
        let data_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&data_path)?;

        let file_len = data_file.metadata()?.len();
        if file_len == 0 {
            data_file.set_len(Self::INITIAL_SIZE)?;
        }

        let mmap = unsafe { MmapMut::map_mut(&data_file)? };

        // 2. Open/Create WAL
        let wal_path = path.join("vectors.wal");
        let wal_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&wal_path)?;
        let wal = io::BufWriter::new(wal_file);

        // 3. Load Index
        let index_path = path.join("vectors.idx");
        let (index, next_offset) = if index_path.exists() {
            let file = File::open(&index_path)?;
            let index: HashMap<u64, usize> = bincode::deserialize_from(io::BufReader::new(file))
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            // Calculate next_offset based on stored data
            // Simple approach: max(offset) + size
            let max_offset = index.values().max().copied().unwrap_or(0);
            let size = if index.is_empty() {
                0
            } else {
                max_offset + dimension * 4
            };
            (index, size)
        } else {
            (HashMap::new(), 0)
        };

        Ok(Self {
            path,
            dimension,
            index: RwLock::new(index),
            wal: RwLock::new(wal),
            data_file,
            mmap: RwLock::new(mmap),
            next_offset: AtomicUsize::new(next_offset),
        })
    }

    /// Ensures the memory map is large enough to hold data at `offset`.
    fn ensure_capacity(&mut self, required_len: usize) -> io::Result<()> {
        let mut mmap = self.mmap.write();
        if mmap.len() < required_len {
            // Flush current mmap before unmapping (handled by drop, but explicit flush is good)
            mmap.flush()?;

            // Calculate new size
            let current_len = mmap.len() as u64;
            let needed_growth = (required_len as u64).saturating_sub(current_len);
            let growth = std::cmp::max(needed_growth, std::cmp::max(Self::MIN_GROWTH, current_len));
            let new_len = current_len + growth;

            // Resize file
            self.data_file.set_len(new_len)?;

            // Remap
            *mmap = unsafe { MmapMut::map_mut(&self.data_file)? };
        }
        Ok(())
    }
}

impl VectorStorage for MmapStorage {
    fn store(&mut self, id: u64, vector: &[f32]) -> io::Result<()> {
        if vector.len() != self.dimension {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Vector dimension mismatch: expected {}, got {}",
                    self.dimension,
                    vector.len()
                ),
            ));
        }

        let vector_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(vector.as_ptr().cast::<u8>(), std::mem::size_of_val(vector))
        };

        // 1. Write to WAL
        {
            let mut wal = self.wal.write();
            // Op: Store (1) | ID | Len | Data
            wal.write_all(&[1u8])?;
            wal.write_all(&id.to_le_bytes())?;
            #[allow(clippy::cast_possible_truncation)]
            let len_u32 = vector_bytes.len() as u32;
            wal.write_all(&len_u32.to_le_bytes())?;
            wal.write_all(vector_bytes)?;
        }

        // 2. Determine offset
        let vector_size = vector_bytes.len();

        let (offset, is_new) = {
            let index = self.index.read();
            if let Some(&existing_offset) = index.get(&id) {
                (existing_offset, false)
            } else {
                let offset = self.next_offset.load(Ordering::Relaxed);
                self.next_offset.fetch_add(vector_size, Ordering::Relaxed);
                (offset, true)
            }
        };

        // Ensure capacity and write
        self.ensure_capacity(offset + vector_size)?;

        {
            let mut mmap = self.mmap.write();
            mmap[offset..offset + vector_size].copy_from_slice(vector_bytes);
        }

        // 3. Update Index if new
        if is_new {
            self.index.write().insert(id, offset);
        }

        Ok(())
    }

    fn store_batch(&mut self, vectors: &[(u64, &[f32])]) -> io::Result<usize> {
        if vectors.is_empty() {
            return Ok(0);
        }

        let vector_size = self.dimension * std::mem::size_of::<f32>();

        // Validate all dimensions upfront
        for (_, vector) in vectors {
            if vector.len() != self.dimension {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Vector dimension mismatch: expected {}, got {}",
                        self.dimension,
                        vector.len()
                    ),
                ));
            }
        }

        // 1. Calculate total space needed and prepare batch WAL entry
        let mut new_vectors: Vec<(u64, usize)> = Vec::with_capacity(vectors.len());
        let mut total_new_size = 0usize;

        {
            let index = self.index.read();
            for &(id, _) in vectors {
                if !index.contains_key(&id) {
                    let offset = self.next_offset.load(Ordering::Relaxed) + total_new_size;
                    new_vectors.push((id, offset));
                    total_new_size += vector_size;
                }
            }
        }

        // 2. Pre-allocate space for all new vectors at once
        if total_new_size > 0 {
            let start_offset = self.next_offset.load(Ordering::Relaxed);
            self.ensure_capacity(start_offset + total_new_size)?;
            self.next_offset
                .fetch_add(total_new_size, Ordering::Relaxed);
        }

        // 3. Single WAL write for entire batch (Op: BatchStore = 3)
        {
            let mut wal = self.wal.write();
            // Batch header: Op(1) | Count(4)
            wal.write_all(&[3u8])?;
            #[allow(clippy::cast_possible_truncation)]
            let count = vectors.len() as u32;
            wal.write_all(&count.to_le_bytes())?;

            // Write all vectors contiguously
            for &(id, vector) in vectors {
                let vector_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        vector.as_ptr().cast::<u8>(),
                        std::mem::size_of_val(vector),
                    )
                };
                wal.write_all(&id.to_le_bytes())?;
                #[allow(clippy::cast_possible_truncation)]
                let len_u32 = vector_bytes.len() as u32;
                wal.write_all(&len_u32.to_le_bytes())?;
                wal.write_all(vector_bytes)?;
            }
            // Note: No flush here - caller controls fsync timing
        }

        // 4. Write all vectors to mmap contiguously
        {
            let index = self.index.read();
            let mut mmap = self.mmap.write();

            for &(id, vector) in vectors {
                let vector_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        vector.as_ptr().cast::<u8>(),
                        std::mem::size_of_val(vector),
                    )
                };

                // Get offset (existing or from new_vectors)
                let offset = if let Some(&existing) = index.get(&id) {
                    existing
                } else {
                    new_vectors
                        .iter()
                        .find(|(vid, _)| *vid == id)
                        .map_or(0, |(_, off)| *off)
                };

                mmap[offset..offset + vector_size].copy_from_slice(vector_bytes);
            }
        }

        // 5. Batch update index
        if !new_vectors.is_empty() {
            let mut index = self.index.write();
            for (id, offset) in new_vectors {
                index.insert(id, offset);
            }
        }

        Ok(vectors.len())
    }

    fn retrieve(&self, id: u64) -> io::Result<Option<Vec<f32>>> {
        let index = self.index.read();
        let Some(&offset) = index.get(&id) else {
            return Ok(None);
        };
        drop(index); // Release lock

        let mmap = self.mmap.read();
        let vector_size = self.dimension * std::mem::size_of::<f32>();

        if offset + vector_size > mmap.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Offset out of bounds",
            ));
        }

        let bytes = &mmap[offset..offset + vector_size];

        // Convert bytes back to f32
        let mut vector = vec![0.0f32; self.dimension];
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                vector.as_mut_ptr().cast::<u8>(),
                vector_size,
            );
        }

        Ok(Some(vector))
    }

    fn delete(&mut self, id: u64) -> io::Result<()> {
        // 1. Write to WAL
        {
            let mut wal = self.wal.write();
            // Op: Delete (2) | ID
            wal.write_all(&[2u8])?;
            wal.write_all(&id.to_le_bytes())?;
        }

        // 2. Remove from Index
        let mut index = self.index.write();
        index.remove(&id);

        // Note: We don't reclaim space in Mmap for MVP. Compaction is needed.

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        // 1. Flush Mmap
        self.mmap.write().flush()?;

        // 2. Flush WAL
        self.wal.write().flush()?;

        // 3. Save Index
        let index_path = self.path.join("vectors.idx");
        let file = File::create(&index_path)?;
        let index = self.index.read();
        bincode::serialize_into(io::BufWriter::new(file), &*index).map_err(io::Error::other)?;

        Ok(())
    }

    fn len(&self) -> usize {
        self.index.read().len()
    }
}

/// Trait defining storage operations for metadata payloads.
pub trait PayloadStorage: Send + Sync {
    /// Stores a payload with the given ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    fn store(&mut self, id: u64, payload: &serde_json::Value) -> io::Result<()>;

    /// Retrieves a payload by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the read operation fails.
    fn retrieve(&self, id: u64) -> io::Result<Option<serde_json::Value>>;

    /// Deletes a payload by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete operation fails.
    fn delete(&mut self, id: u64) -> io::Result<()>;

    /// Flushes pending writes to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush operation fails.
    fn flush(&mut self) -> io::Result<()>;

    /// Returns all stored IDs.
    fn ids(&self) -> Vec<u64>;
}

/// Log-structured payload storage.
///
/// Stores payloads in an append-only log file with an in-memory index.
#[allow(clippy::module_name_repetitions)]
pub struct LogPayloadStorage {
    _path: PathBuf,
    index: RwLock<HashMap<u64, u64>>, // ID -> Offset of length
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
        let mut index = HashMap::new();
        let len = reader.metadata()?.len();
        let mut pos = 0;
        let mut reader_buf = io::BufReader::new(&reader);

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn test_storage_new_creates_files() {
        let dir = tempdir().unwrap();
        let storage = MmapStorage::new(dir.path(), 3).unwrap();

        assert!(dir.path().join("vectors.dat").exists());
        assert!(dir.path().join("vectors.wal").exists());
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn test_storage_store_and_retrieve() {
        let dir = tempdir().unwrap();
        let mut storage = MmapStorage::new(dir.path(), 3).unwrap();
        let vector = vec![1.0, 2.0, 3.0];

        storage.store(1, &vector).unwrap();

        let retrieved = storage.retrieve(1).unwrap();
        assert_eq!(retrieved, Some(vector));
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn test_storage_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let vector = vec![1.0, 2.0, 3.0];

        {
            let mut storage = MmapStorage::new(&path, 3).unwrap();
            storage.store(1, &vector).unwrap();
            storage.flush().unwrap();
        } // storage dropped

        // Re-open
        let storage = MmapStorage::new(&path, 3).unwrap();
        let retrieved = storage.retrieve(1).unwrap();
        assert_eq!(retrieved, Some(vector));
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn test_storage_delete() {
        let dir = tempdir().unwrap();
        let mut storage = MmapStorage::new(dir.path(), 3).unwrap();
        let vector = vec![1.0, 2.0, 3.0];

        storage.store(1, &vector).unwrap();
        storage.delete(1).unwrap();

        let retrieved = storage.retrieve(1).unwrap();
        assert_eq!(retrieved, None);
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn test_storage_wal_recovery() {
        let dir = tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let vector = vec![1.0, 2.0, 3.0];

        {
            let mut storage = MmapStorage::new(&path, 3).unwrap();
            storage.store(1, &vector).unwrap();
            // Manual flush to ensure index is saved for MVP persistence
            storage.flush().unwrap();
        }

        // Re-open
        let storage = MmapStorage::new(&path, 3).unwrap();
        let retrieved = storage.retrieve(1).unwrap();
        assert_eq!(retrieved, Some(vector));
    }

    #[test]
    fn test_payload_storage_new() {
        let dir = tempdir().unwrap();
        let _storage = LogPayloadStorage::new(dir.path()).unwrap();
        assert!(dir.path().join("payloads.log").exists());
    }

    #[test]
    fn test_payload_storage_ops() {
        let dir = tempdir().unwrap();
        let mut storage = LogPayloadStorage::new(dir.path()).unwrap();
        let payload = json!({"key": "value", "num": 42});

        // Store
        storage.store(1, &payload).unwrap();

        // Retrieve
        let retrieved = storage.retrieve(1).unwrap();
        assert_eq!(retrieved, Some(payload.clone()));

        // Delete
        storage.delete(1).unwrap();
        let retrieved = storage.retrieve(1).unwrap();
        assert_eq!(retrieved, None);
    }

    #[test]
    fn test_payload_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let payload = json!({"foo": "bar"});

        {
            let mut storage = LogPayloadStorage::new(&path).unwrap();
            storage.store(1, &payload).unwrap();
            storage.flush().unwrap();
        }

        let storage = LogPayloadStorage::new(&path).unwrap();
        let retrieved = storage.retrieve(1).unwrap();
        assert_eq!(retrieved, Some(payload));
    }
}
