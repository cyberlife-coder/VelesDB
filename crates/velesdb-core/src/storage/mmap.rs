//! Memory-mapped file storage for vectors.
//!
//! Uses a combination of an index file (ID -> offset) and a data file (raw vectors).
//! Also implements a simple WAL for durability.
//!
//! # Safety Guarantees (EPIC-032/US-001)
//!
//! All vector data is stored with f32 alignment (4 bytes):
//! - Initial offset starts at 0 (aligned)
//! - Each vector occupies `dimension * 4` bytes (always a multiple of 4)
//! - Offsets are verified at runtime before pointer casting
//!
//! # P2 Optimization: Aggressive Pre-allocation
//!
//! To minimize blocking during `ensure_capacity` (which requires a write lock),
//! we use aggressive pre-allocation:
//! - Initial size: 16MB (vs 64KB before) - handles most small-medium datasets
//! - Growth factor: 2x minimum with 64MB floor - fewer resize operations
//! - Explicit `reserve_capacity()` for bulk imports

use super::compaction::CompactionContext;
use super::guard::VectorSliceGuard;
use super::metrics::StorageMetrics;
use super::sharded_index::ShardedIndex;
use super::traits::VectorStorage;
use super::vector_bytes::{bytes_to_vector, vector_to_bytes};

use memmap2::MmapMut;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tracing::error;

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
    /// EPIC-033/US-004: Sharded for reduced lock contention on read-heavy workloads
    index: ShardedIndex,
    /// Write-Ahead Log writer
    wal: RwLock<io::BufWriter<File>>,
    /// File handle for the data file (kept open for resizing)
    data_file: File,
    /// Memory mapped data file
    mmap: RwLock<MmapMut>,
    /// Next available offset in the data file
    next_offset: AtomicUsize,
    /// P0 Audit: Metrics for monitoring `ensure_capacity` latency
    metrics: Arc<StorageMetrics>,
    /// Epoch counter incremented every time the mmap is remapped.
    ///
    /// # Overflow Safety
    ///
    /// Uses wrapping arithmetic (guaranteed by `fetch_add`). Even at 1 billion
    /// remaps/second, overflow would take ~584 years. The worst-case scenario
    /// on wrap is a false-positive panic in `VectorSliceGuard::as_slice()`,
    /// which is acceptable given the astronomical time required.
    remap_epoch: AtomicU64,
}

impl MmapStorage {
    /// P2: Increased from 64KB to 16MB for better initial capacity.
    /// This handles most small-medium datasets without any resize operations.
    const INITIAL_SIZE: u64 = 16 * 1024 * 1024; // 16MB initial size

    /// P2: Increased from 1MB to 64MB minimum growth.
    /// Fewer resize operations = fewer blocking write locks.
    const MIN_GROWTH: u64 = 64 * 1024 * 1024; // Minimum 64MB growth

    /// P2: Growth factor for exponential pre-allocation.
    /// Each resize at least doubles capacity for amortized O(1) growth.
    const GROWTH_FACTOR: u64 = 2;

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

        // SAFETY: data_file is a valid, open file with set_len() called to ensure
        // the mapping range is fully allocated. MmapMut requires the file to be
        // readable and writable, which is guaranteed by OpenOptions above.
        let mmap = unsafe { MmapMut::map_mut(&data_file)? };

        // 2. Open/Create WAL
        let wal_path = path.join("vectors.wal");
        let wal_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&wal_path)?;
        let wal = io::BufWriter::new(wal_file);

        // 3. Load Index (EPIC-033/US-004: Convert to ShardedIndex)
        let index_path = path.join("vectors.idx");
        let (index, next_offset) = if index_path.exists() {
            let file = File::open(&index_path)?;
            let flat_index: FxHashMap<u64, usize> =
                bincode::deserialize_from(io::BufReader::new(file))
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            // Calculate next_offset based on stored data
            let max_offset = flat_index.values().max().copied().unwrap_or(0);
            let size = if flat_index.is_empty() {
                0
            } else {
                max_offset + dimension * 4
            };

            // Convert to ShardedIndex
            (ShardedIndex::from_hashmap(flat_index), size)
        } else {
            (ShardedIndex::new(), 0)
        };

        Ok(Self {
            path,
            dimension,
            index,
            wal: RwLock::new(wal),
            data_file,
            mmap: RwLock::new(mmap),
            next_offset: AtomicUsize::new(next_offset),
            metrics: Arc::new(StorageMetrics::new()),
            remap_epoch: AtomicU64::new(0),
        })
    }

    /// Ensures the memory map is large enough to hold data at `offset`.
    ///
    /// # P2 Optimization
    ///
    /// Uses aggressive pre-allocation to minimize blocking:
    /// - Exponential growth (2x) for amortized O(1)
    /// - 64MB minimum growth to reduce resize frequency
    /// - For 1M vectors × 768D × 4 bytes = 3GB, only ~6 resizes needed
    ///
    /// # P0 Audit: Latency Monitoring
    ///
    /// This operation is instrumented to track latency. Monitor P99 latency
    /// via `metrics()` to detect "stop-the-world" pauses during large resizes.
    fn ensure_capacity(&mut self, required_len: usize) -> io::Result<()> {
        let start = Instant::now();
        let mut did_resize = false;
        let mut bytes_resized = 0u64;

        let mut mmap = self.mmap.write();
        if mmap.len() < required_len {
            // Flush current mmap before unmapping
            mmap.flush()?;

            // P2: Aggressive pre-allocation strategy
            // Calculate new size with exponential growth
            let current_len = mmap.len() as u64;
            let required_u64 = required_len as u64;

            // Option 1: Double current size (exponential growth)
            let doubled = current_len.saturating_mul(Self::GROWTH_FACTOR);
            // Option 2: Required + MIN_GROWTH headroom
            let with_headroom = required_u64.saturating_add(Self::MIN_GROWTH);
            // Option 3: Just the minimum growth
            let min_growth = current_len.saturating_add(Self::MIN_GROWTH);

            // Take the maximum to ensure both sufficient space and good amortization
            let new_len = doubled.max(with_headroom).max(min_growth).max(required_u64);

            // Resize file
            self.data_file.set_len(new_len)?;

            // SAFETY: data_file has been resized with set_len(new_len) above,
            // ensuring the new mapping range is fully allocated. The old mmap
            // is dropped when we assign the new one.
            *mmap = unsafe { MmapMut::map_mut(&self.data_file)? };
            // Increment epoch so existing VectorSliceGuards become invalid
            self.remap_epoch.fetch_add(1, Ordering::Release);

            did_resize = true;
            bytes_resized = new_len.saturating_sub(current_len);
        }

        // P0 Audit: Record latency metrics
        self.metrics
            .record_ensure_capacity(start.elapsed(), did_resize, bytes_resized);

        Ok(())
    }

    /// Pre-allocates storage capacity for a known number of vectors.
    ///
    /// Call this before bulk imports to avoid blocking resize operations
    /// during insertion. This is especially useful when the final dataset
    /// size is known in advance.
    ///
    /// # P2 Optimization
    ///
    /// This allows users to pre-allocate once and avoid all resize locks
    /// during bulk import operations.
    ///
    /// # Arguments
    ///
    /// * `vector_count` - Expected number of vectors to store
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Pre-allocate for 1 million vectors before bulk import
    /// storage.reserve_capacity(1_000_000)?;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn reserve_capacity(&mut self, vector_count: usize) -> io::Result<()> {
        let vector_size = self.dimension * std::mem::size_of::<f32>();
        let required_len = vector_count.saturating_mul(vector_size);

        // Add 10% headroom for safety
        let with_headroom = required_len.saturating_add(required_len / 10);

        self.ensure_capacity(with_headroom)
    }

    /// Returns a reference to the storage metrics.
    ///
    /// # P0 Audit: Latency Monitoring
    ///
    /// Use this to monitor `ensure_capacity` latency, especially P99.
    /// High P99 latency indicates "stop-the-world" pauses during mmap resizes.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let storage = MmapStorage::new(path, 768)?;
    /// // ... perform operations ...
    /// let stats = storage.metrics().ensure_capacity_latency_stats();
    /// if stats.p99_exceeds(Duration::from_millis(100)) {
    ///     warn!("High P99 latency detected: {:?}", stats.p99());
    /// }
    /// ```
    #[must_use]
    pub fn metrics(&self) -> &StorageMetrics {
        &self.metrics
    }

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

        // CRITICAL FIX: After compaction, data_file must point to the new file.
        // CompactionContext::compact() atomically replaces vectors.dat via rename(),
        // and remaps self.mmap to the new file. However, it cannot update data_file
        // because it doesn't have access to it. We must reopen data_file here to
        // ensure future resize operations (ensure_capacity) work on the correct file.
        if bytes_reclaimed > 0 {
            let data_path = self.path.join("vectors.dat");
            self.data_file = OpenOptions::new().read(true).write(true).open(&data_path)?;

            self.flush()?;
        }

        Ok(bytes_reclaimed)
    }

    /// Returns the fragmentation ratio (0.0 = no fragmentation, 1.0 = 100% fragmented).
    ///
    /// Use this to decide when to trigger compaction.
    /// A ratio > 0.3 (30% fragmentation) is a good threshold.
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

    /// Retrieves a vector by ID without copying (zero-copy).
    ///
    /// Returns a guard providing direct mmap access. Faster than `retrieve()`
    /// as it eliminates heap allocation and memcpy. Guard must be dropped to release lock.
    ///
    /// # Errors
    ///
    /// Returns an error if the stored offset is out of bounds.
    ///
    /// # Panics
    ///
    /// Panics if the stored offset is not f32-aligned (must be multiple of 4).
    /// This should never happen with properly stored data.
    pub fn retrieve_ref(&self, id: u64) -> io::Result<Option<VectorSliceGuard<'_>>> {
        // EPIC-033/US-004: Use sharded index for reduced contention
        let offset = match self.index.get(id) {
            Some(offset) => offset,
            None => return Ok(None),
        };

        // Now acquire mmap read lock and validate bounds
        let mmap = self.mmap.read();
        let vector_size = self.dimension * std::mem::size_of::<f32>();

        if offset + vector_size > mmap.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Offset out of bounds",
            ));
        }

        // EPIC-032/US-001: Verify alignment before pointer cast
        // SAFETY: We've validated that:
        // 1. offset + vector_size <= mmap.len() (bounds check above)
        // 2. offset is 4-byte aligned (assertion below - enforced in release too)
        // 3. The pointer is derived from the mmap which is held by the guard
        // 4. All writes via store() use f32-aligned offsets (dimension * 4)
        // P2 Audit 2026-01-29: Converted from debug_assert to assert for memory safety
        assert!(
            offset % std::mem::align_of::<f32>() == 0,
            "EPIC-032/US-001: offset {} is not f32-aligned (must be multiple of {})",
            offset,
            std::mem::align_of::<f32>()
        );
        #[allow(clippy::cast_ptr_alignment)]
        let ptr = unsafe { mmap.as_ptr().add(offset).cast::<f32>() };

        let epoch_at_creation = self.remap_epoch.load(Ordering::Acquire);
        Ok(Some(VectorSliceGuard {
            _guard: mmap,
            ptr,
            len: self.dimension,
            epoch_ptr: &self.remap_epoch,
            epoch_at_creation,
        }))
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

        let vector_bytes = vector_to_bytes(vector);

        // 1. Write to WAL
        {
            let mut wal = self.wal.write();
            // Op: Store (1) | ID | Len | Data
            wal.write_all(&[1u8])?;
            wal.write_all(&id.to_le_bytes())?;
            // SAFETY: Vector byte length is dimension * 4 bytes. With max dimension 65536,
            // max bytes = 262144 which fits in u32 (max 4,294,967,295)
            #[allow(clippy::cast_possible_truncation)]
            let len_u32 = vector_bytes.len() as u32;
            wal.write_all(&len_u32.to_le_bytes())?;
            wal.write_all(vector_bytes)?;
        }

        // 2. Determine offset (EPIC-033/US-004: Use sharded index)
        let vector_size = vector_bytes.len();

        let (offset, is_new) = if let Some(existing_offset) = self.index.get(id) {
            (existing_offset, false)
        } else {
            let offset = self.next_offset.load(Ordering::Relaxed);
            self.next_offset.fetch_add(vector_size, Ordering::Relaxed);
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
        // Perf: Use FxHashMap for O(1) lookup instead of Vec with O(n) find
        // EPIC-033/US-004: Use sharded index for reduced contention
        let mut new_vector_offsets: FxHashMap<u64, usize> = FxHashMap::default();
        new_vector_offsets.reserve(vectors.len());
        let mut total_new_size = 0usize;

        for &(id, _) in vectors {
            if !self.index.contains_key(id) {
                let offset = self.next_offset.load(Ordering::Relaxed) + total_new_size;
                new_vector_offsets.insert(id, offset);
                total_new_size += vector_size;
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
            // SAFETY: Batch size is limited by memory constraints. In practice, batches
            // are < 100K vectors which fits in u32 (max 4,294,967,295)
            #[allow(clippy::cast_possible_truncation)]
            let count = vectors.len() as u32;
            wal.write_all(&count.to_le_bytes())?;

            // Write all vectors contiguously
            for &(id, vector) in vectors {
                let vector_bytes = vector_to_bytes(vector);
                wal.write_all(&id.to_le_bytes())?;
                // SAFETY: Vector byte length is dimension * 4 bytes. With max dimension 65536,
                // max bytes = 262144 which fits in u32 (max 4,294,967,295)
                #[allow(clippy::cast_possible_truncation)]
                let len_u32 = vector_bytes.len() as u32;
                wal.write_all(&len_u32.to_le_bytes())?;
                wal.write_all(vector_bytes)?;
            }
            // Note: No flush here - caller controls fsync timing
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
                    new_vector_offsets.get(&id).copied().unwrap_or(0)
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
        // 1. Write to WAL
        {
            let mut wal = self.wal.write();
            // Op: Delete (2) | ID
            wal.write_all(&[2u8])?;
            wal.write_all(&id.to_le_bytes())?;
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
            let offset_u64 = u64::try_from(offset).expect("offset fits in u64");
            let size_u64 = u64::try_from(vector_size).expect("vector_size fits in u64");
            let _ = super::compaction::punch_hole(&self.data_file, offset_u64, size_u64);
        }

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
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
        bincode::serialize_into(&mut writer, &flat_index).map_err(io::Error::other)?;
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

// -----------------------------------------------------------------------------
// Drop implementation – guarantees durability on graceful shutdown
// -----------------------------------------------------------------------------
impl Drop for MmapStorage {
    #[allow(clippy::cognitive_complexity)] // Reason: Drop must handle WAL+mmap flush atomically, splitting risks data loss
    fn drop(&mut self) {
        // 1. Flush WAL first (durability of operation log)
        if let Some(mut wal) = self.wal.try_write() {
            if let Err(e) = wal.flush() {
                error!(?e, "Failed to flush WAL in MmapStorage::drop");
            }
            if let Err(e) = wal.get_ref().sync_all() {
                error!(?e, "Failed to fsync WAL in MmapStorage::drop");
            }
        }

        // 2. Flush mmap to persist vector bytes
        if let Some(mmap) = self.mmap.try_write() {
            if let Err(e) = mmap.flush() {
                error!(?e, "Failed to flush mmap in MmapStorage::drop");
            }
        }
    }
}
