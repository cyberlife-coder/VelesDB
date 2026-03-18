//! `VectorStorage` trait implementation for `MmapStorage`.
//!
//! Extracted from `mmap.rs` for maintainability (04-05 module splitting).
//! Handles store, retrieve, delete, flush, and batch operations.
//!
//! # Durability Contract
//!
//! `store`/`store_batch` append to WAL and update mmap, but callers must invoke
//! `flush()` when they need a deterministic durability barrier.
//! `Drop` only performs best-effort sync and must not be relied on as a commit point.

use super::MmapStorage;
use crate::storage::log_payload::crc32_hash;
use crate::storage::traits::VectorStorage;
use crate::storage::vector_bytes::{bytes_to_vector, vector_to_bytes};

use rustc_hash::FxHashMap;
use std::fs::File;
use std::io::{self, Write};
use std::sync::atomic::Ordering;

/// Writes a CRC32-framed store entry to the WAL.
///
/// Format: `[op:1][id:8][len:4][data:N][crc32:4]`
///
/// The CRC32 covers `op + id + len + data`.
fn write_wal_store_entry(wal: &mut io::BufWriter<File>, id: u64, data: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(1 + 8 + 4 + data.len());
    frame.push(1u8);
    frame.extend_from_slice(&id.to_le_bytes());
    // Reason: Vector byte length is dimension * 4. With max dimension 65536,
    // max bytes = 262144 which fits in u32 (max 4,294,967,295).
    #[allow(clippy::cast_possible_truncation)]
    let len_u32 = data.len() as u32;
    frame.extend_from_slice(&len_u32.to_le_bytes());
    frame.extend_from_slice(data);
    let crc = crc32_hash(&frame);
    wal.write_all(&frame)?;
    wal.write_all(&crc.to_le_bytes())
}

/// Writes a CRC32-framed delete entry to the WAL.
///
/// Format: `[op:1][id:8][crc32:4]`
///
/// The CRC32 covers `op + id`.
fn write_wal_delete_entry(wal: &mut io::BufWriter<File>, id: u64) -> io::Result<()> {
    let mut frame = Vec::with_capacity(1 + 8);
    frame.push(2u8);
    frame.extend_from_slice(&id.to_le_bytes());
    let crc = crc32_hash(&frame);
    wal.write_all(&frame)?;
    wal.write_all(&crc.to_le_bytes())
}

/// RF-2: Shared dimension validation for `store` and `store_batch`.
#[inline]
fn validate_dimension(expected: usize, actual: usize) -> io::Result<()> {
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Vector dimension mismatch: expected {expected}, got {actual}"),
        ));
    }
    Ok(())
}

impl VectorStorage for MmapStorage {
    fn store(&mut self, id: u64, vector: &[f32]) -> io::Result<()> {
        validate_dimension(self.dimension, vector.len())?;

        let vector_bytes = vector_to_bytes(vector);

        // 1. Write to WAL with CRC32 framing (Issue #317)
        {
            let mut wal = self.wal.write();
            write_wal_store_entry(&mut wal, id, vector_bytes)?;
        }

        // 2. Determine offset (EPIC-033/US-004: Use sharded index)
        let vector_size = vector_bytes.len();

        let (offset, is_new) = if let Some(existing_offset) = self.index.get(id) {
            (existing_offset, false)
        } else {
            // M-2: Use Acquire/AcqRel to ensure mmap writes are visible on ARM/RISC-V.
            let offset = self.next_offset.load(Ordering::Acquire);
            self.next_offset.fetch_add(vector_size, Ordering::AcqRel);
            (offset, true)
        };

        // Ensure capacity and write
        self.ensure_capacity(offset + vector_size)?;

        {
            let mut mmap = self.mmap.write();
            mmap[offset..offset + vector_size].copy_from_slice(vector_bytes);
        }

        // 3. Update Index if new (EPIC-033/US-004: Use sharded index)
        if is_new {
            self.index.insert(id, offset);
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
            validate_dimension(self.dimension, vector.len())?;
        }

        // 1. Calculate total space needed and prepare batch WAL entry
        // Perf: Use FxHashMap for O(1) lookup instead of Vec with O(n) find
        // EPIC-033/US-004: Use sharded index for reduced contention
        let mut new_vector_offsets: FxHashMap<u64, usize> = FxHashMap::default();
        new_vector_offsets.reserve(vectors.len());
        let mut total_new_size = 0usize;

        for &(id, _) in vectors {
            if !self.index.contains_key(id) {
                // M-2: Acquire ordering for cross-platform visibility
                let offset = self.next_offset.load(Ordering::Acquire) + total_new_size;
                new_vector_offsets.insert(id, offset);
                total_new_size += vector_size;
            }
        }

        // 2. Pre-allocate space for all new vectors at once
        if total_new_size > 0 {
            // M-2: Acquire/AcqRel ordering for cross-platform visibility
            let start_offset = self.next_offset.load(Ordering::Acquire);
            self.ensure_capacity(start_offset + total_new_size)?;
            self.next_offset.fetch_add(total_new_size, Ordering::AcqRel);
        }

        // 3. WAL append with CRC32 framing per entry (Issue #317)
        {
            let mut wal = self.wal.write();
            for &(id, vector) in vectors {
                let vector_bytes = vector_to_bytes(vector);
                write_wal_store_entry(&mut wal, id, vector_bytes)?;
            }
            // Note: no fsync here, caller controls durability via `flush()`.
        }

        // 4. Write all vectors to mmap contiguously
        // EPIC-033/US-004: Use sharded index for reduced contention
        {
            let mut mmap = self.mmap.write();

            for &(id, vector) in vectors {
                let vector_bytes = vector_to_bytes(vector);

                // Get offset (existing or from new_vector_offsets)
                // Perf: O(1) HashMap lookup instead of O(n) linear search
                let offset = if let Some(existing) = self.index.get(id) {
                    existing
                } else {
                    // BUG FIX (04-05): Replaced `unwrap_or(0)` which would silently
                    // write to offset 0 (corrupting the first vector) if the invariant
                    // ever broke. Every new ID is added to `new_vector_offsets` above,
                    // so this should always succeed.
                    new_vector_offsets.get(&id).copied().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "ID not found in new_vector_offsets",
                        )
                    })?
                };

                mmap[offset..offset + vector_size].copy_from_slice(vector_bytes);
            }
        }

        // 5. Batch update index (EPIC-033/US-004: Use sharded index)
        for (id, offset) in new_vector_offsets {
            self.index.insert(id, offset);
        }

        Ok(vectors.len())
    }

    fn retrieve(&self, id: u64) -> io::Result<Option<Vec<f32>>> {
        // EPIC-033/US-004: Use sharded index for reduced contention
        let Some(offset) = self.index.get(id) else {
            return Ok(None);
        };

        let mmap = self.mmap.read();
        let vector_size = self.dimension * std::mem::size_of::<f32>();

        if offset + vector_size > mmap.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Offset out of bounds",
            ));
        }

        let bytes = &mmap[offset..offset + vector_size];
        Ok(Some(bytes_to_vector(bytes, self.dimension)))
    }

    fn delete(&mut self, id: u64) -> io::Result<()> {
        // 1. Write to WAL with CRC32 framing (Issue #317)
        {
            let mut wal = self.wal.write();
            write_wal_delete_entry(&mut wal, id)?;
        }

        // 2. Get offset before removing from index (for hole-punch)
        // EPIC-033/US-004: Use sharded index for reduced contention
        let offset = self.index.get(id);

        // 3. Remove from Index
        self.index.remove(id);

        // 4. EPIC-033/US-003: Hole-punch to reclaim disk space immediately
        // This releases disk blocks back to the filesystem without rewriting the file
        if let Some(offset) = offset {
            let vector_size = self.dimension * std::mem::size_of::<f32>();
            // Best-effort: ignore errors (space will be reclaimed on compact())
            // Reason: offset and vector_size are bounded by file size, always fit in u64 on 64-bit
            let offset_u64 = u64::try_from(offset).unwrap_or(u64::MAX);
            let size_u64 = u64::try_from(vector_size).unwrap_or(u64::MAX);
            if offset_u64 != u64::MAX && size_u64 != u64::MAX {
                let _ =
                    crate::storage::compaction::punch_hole(&self.data_file, offset_u64, size_u64);
            }
        }

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Explicit durability barrier:
        // 1) flush mmap bytes, 2) flush+fsync WAL, 3) persist+fsync index file.
        // Callers requiring durable state across crashes should call this method.
        // 1. Flush Mmap
        self.mmap.write().flush()?;

        // 2. Flush WAL and fsync for durability
        {
            let mut wal = self.wal.write();
            wal.flush()?;
            wal.get_ref().sync_all()?;
        }

        // 3. Save Index (EPIC-033/US-004: Convert ShardedIndex to flat HashMap for serialization)
        // EPIC-069/US-001: fsync index file for crash recovery on Windows
        let index_path = self.path.join("vectors.idx");
        let file = File::create(&index_path)?;
        let mut writer = io::BufWriter::new(file);
        let flat_index = self.index.to_hashmap();
        let bytes = postcard::to_allocvec(&flat_index).map_err(io::Error::other)?;
        writer.write_all(&bytes)?;
        writer.flush()?;
        writer
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)?
            .sync_all()?;

        Ok(())
    }

    fn len(&self) -> usize {
        self.index.len()
    }

    fn ids(&self) -> Vec<u64> {
        self.index.keys()
    }
}
