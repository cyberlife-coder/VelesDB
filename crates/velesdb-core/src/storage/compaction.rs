//! Storage compaction for reclaiming space from deleted vectors.
//!
//! This module provides compaction functionality for `MmapStorage`,
//! allowing reclamation of disk space from deleted vectors.

use memmap2::MmapMut;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Compaction configuration and state.
pub(super) struct CompactionContext<'a> {
    pub path: &'a Path,
    pub dimension: usize,
    pub index: &'a RwLock<FxHashMap<u64, usize>>,
    pub mmap: &'a RwLock<MmapMut>,
    pub next_offset: &'a AtomicUsize,
    pub wal: &'a RwLock<io::BufWriter<File>>,
    pub initial_size: u64,
}

impl CompactionContext<'_> {
    /// Compacts the storage by rewriting only active vectors.
    ///
    /// This reclaims disk space from deleted vectors by:
    /// 1. Writing all active vectors to a new temporary file
    /// 2. Atomically replacing the old file with the new one
    ///
    /// # TS-CORE-004: Storage Compaction
    ///
    /// This operation is quasi-atomic via `rename()` for crash safety.
    /// Reads remain available during compaction (copy-on-write pattern).
    ///
    /// # Returns
    ///
    /// The number of bytes reclaimed.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn compact(&self) -> io::Result<usize> {
        let vector_size = self.dimension * std::mem::size_of::<f32>();

        // 1. Get current state
        let index = self.index.read();
        let active_count = index.len();

        if active_count == 0 {
            drop(index);
            return Ok(0);
        }

        // Calculate space used vs allocated
        let current_offset = self.next_offset.load(Ordering::Relaxed);
        let active_size = active_count * vector_size;

        if current_offset <= active_size {
            drop(index);
            return Ok(0);
        }

        let bytes_to_reclaim = current_offset - active_size;

        // 2. Create temporary file for compacted data
        let temp_path = self.path.join("vectors.dat.tmp");
        let temp_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;

        // Size the temp file for active vectors
        let new_size = (active_size as u64).max(self.initial_size);
        temp_file.set_len(new_size)?;

        // SAFETY: temp_file is a valid, newly created file with set_len() called
        let mut temp_mmap = unsafe { MmapMut::map_mut(&temp_file)? };

        // 3. Copy active vectors to new file with new offsets
        let mmap = self.mmap.read();
        let mut new_index: FxHashMap<u64, usize> = FxHashMap::default();
        new_index.reserve(active_count);

        let mut new_offset = 0usize;
        for (&id, &old_offset) in index.iter() {
            let src = &mmap[old_offset..old_offset + vector_size];
            temp_mmap[new_offset..new_offset + vector_size].copy_from_slice(src);
            new_index.insert(id, new_offset);
            new_offset += vector_size;
        }

        drop(mmap);
        drop(index);

        // 4. Flush temp file
        temp_mmap.flush()?;
        drop(temp_mmap);
        drop(temp_file);

        // 5. Atomic swap: rename temp -> main
        let data_path = self.path.join("vectors.dat");
        std::fs::rename(&temp_path, &data_path)?;

        // 6. Reopen the compacted file
        let new_data_file = OpenOptions::new().read(true).write(true).open(&data_path)?;
        // SAFETY: new_data_file is the compacted file just renamed from temp
        let new_mmap = unsafe { MmapMut::map_mut(&new_data_file)? };

        // 7. Update internal state
        *self.mmap.write() = new_mmap;
        *self.index.write() = new_index;
        self.next_offset.store(new_offset, Ordering::Relaxed);

        // 8. Write compaction marker to WAL
        {
            let mut wal = self.wal.write();
            wal.write_all(&[4u8])?;
            wal.flush()?;
        }

        Ok(bytes_to_reclaim)
    }

    /// Returns the fragmentation ratio (0.0 = no fragmentation, 1.0 = 100% fragmented).
    ///
    /// Use this to decide when to trigger compaction.
    /// A ratio > 0.3 (30% fragmentation) is a good threshold.
    #[must_use]
    pub fn fragmentation_ratio(&self) -> f64 {
        let index = self.index.read();
        let active_count = index.len();
        drop(index);

        if active_count == 0 {
            return 0.0;
        }

        let vector_size = self.dimension * std::mem::size_of::<f32>();
        let active_size = active_count * vector_size;
        let current_offset = self.next_offset.load(Ordering::Relaxed);

        if current_offset == 0 {
            return 0.0;
        }

        #[allow(clippy::cast_precision_loss)]
        let ratio = 1.0 - (active_size as f64 / current_offset as f64);
        ratio.max(0.0)
    }
}
