//! `MmapStorage` capacity management and compaction.
//!
//! Extracted from `mmap.rs` to reduce NLOC below the 500 threshold.

use super::compaction::CompactionContext;
use super::mmap::MmapStorage;

use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::io;
use std::time::Instant;

impl MmapStorage {
    /// Ensures the memory map is large enough to hold data at `offset`.
    ///
    /// # P2 Optimization
    ///
    /// Uses aggressive pre-allocation to minimize blocking:
    /// - Exponential growth (2x) for amortized O(1)
    /// - 64MB minimum growth to reduce resize frequency
    pub(crate) fn ensure_capacity(&mut self, required_len: usize) -> io::Result<()> {
        let start = Instant::now();
        let mut did_resize = false;
        let mut bytes_resized = 0u64;

        let mut mmap = self.mmap.write();
        if mmap.len() < required_len {
            mmap.flush()?;

            let current_len = mmap.len() as u64;
            let required_u64 = required_len as u64;

            let doubled = current_len.saturating_mul(Self::GROWTH_FACTOR);
            let with_headroom = required_u64.saturating_add(Self::MIN_GROWTH);
            let min_growth = current_len.saturating_add(Self::MIN_GROWTH);

            let new_len = doubled.max(with_headroom).max(min_growth).max(required_u64);

            self.data_file.set_len(new_len)?;

            // SAFETY: data_file has been resized with set_len(new_len) above,
            // ensuring the new mapping range is fully allocated.
            // - Condition 1: File was resized to new_len before remapping.
            // - Condition 2: Old mmap is dropped when we assign the new one.
            // - Condition 3: File remains open with read+write permissions.
            // Reason: Memory mapping requires unsafe; resizing ensures mapping doesn't exceed file bounds.
            *mmap = unsafe { MmapMut::map_mut(&self.data_file)? };
            self.remap_epoch
                .fetch_add(1, std::sync::atomic::Ordering::Release);

            did_resize = true;
            bytes_resized = new_len.saturating_sub(current_len);
        }

        self.metrics
            .record_ensure_capacity(start.elapsed(), did_resize, bytes_resized);

        Ok(())
    }

    /// Pre-allocates storage capacity for a known number of vectors.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn reserve_capacity(&mut self, vector_count: usize) -> io::Result<()> {
        let vector_size = self.dimension * std::mem::size_of::<f32>();
        let required_len = vector_count.saturating_mul(vector_size);
        let with_headroom = required_len.saturating_add(required_len / 10);
        self.ensure_capacity(with_headroom)
    }

    /// Compacts the storage by rewriting only active vectors.
    ///
    /// # Returns
    ///
    /// The number of bytes reclaimed.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn compact(&mut self) -> io::Result<usize> {
        let ctx = CompactionContext {
            path: &self.path,
            dimension: self.dimension,
            index: &self.index,
            mmap: &self.mmap,
            next_offset: &self.next_offset,
            wal: &self.wal,
            initial_size: Self::INITIAL_SIZE,
        };

        let bytes_reclaimed = ctx.compact()?;

        if bytes_reclaimed > 0 {
            let data_path = self.path.join("vectors.dat");
            self.data_file = OpenOptions::new().read(true).write(true).open(&data_path)?;
            self.flush_full()?;
        }

        Ok(bytes_reclaimed)
    }

    /// Returns the fragmentation ratio (0.0 = no fragmentation, 1.0 = 100% fragmented).
    #[must_use]
    pub fn fragmentation_ratio(&self) -> f64 {
        let ctx = CompactionContext {
            path: &self.path,
            dimension: self.dimension,
            index: &self.index,
            mmap: &self.mmap,
            next_offset: &self.next_offset,
            wal: &self.wal,
            initial_size: Self::INITIAL_SIZE,
        };

        ctx.fragmentation_ratio()
    }
}
