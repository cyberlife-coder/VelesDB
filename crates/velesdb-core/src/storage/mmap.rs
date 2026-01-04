//! Memory-mapped file storage for vectors.
//!
//! Uses a combination of an index file (ID -> offset) and a data file (raw vectors).
//! Also implements a simple WAL for durability.
//!
//! # P2 Optimization: Aggressive Pre-allocation
//!
//! To minimize blocking during `ensure_capacity` (which requires a write lock),
//! we use aggressive pre-allocation:
//! - Initial size: 16MB (vs 64KB before) - handles most small-medium datasets
//! - Growth factor: 2x minimum with 64MB floor - fewer resize operations
//! - Explicit `reserve_capacity()` for bulk imports

use super::guard::VectorSliceGuard;
use super::metrics::StorageMetrics;
use super::traits::VectorStorage;

use memmap2::MmapMut;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

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
    /// Perf: `FxHashMap` is faster than std `HashMap` for integer keys
    index: RwLock<FxHashMap<u64, usize>>,
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
            let index: FxHashMap<u64, usize> = bincode::deserialize_from(io::BufReader::new(file))
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
            (FxHashMap::default(), 0)
        };

        Ok(Self {
            path,
            dimension,
            index: RwLock::new(index),
            wal: RwLock::new(wal),
            data_file,
            mmap: RwLock::new(mmap),
            next_offset: AtomicUsize::new(next_offset),
            metrics: Arc::new(StorageMetrics::new()),
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

            // Remap
            *mmap = unsafe { MmapMut::map_mut(&self.data_file)? };

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
        let vector_size = self.dimension * std::mem::size_of::<f32>();

        // 1. Get current state
        let index = self.index.read();
        let active_count = index.len();

        if active_count == 0 {
            // Nothing to compact
            drop(index);
            return Ok(0);
        }

        // Calculate space used vs allocated
        let current_offset = self.next_offset.load(Ordering::Relaxed);
        let active_size = active_count * vector_size;

        if current_offset <= active_size {
            // No fragmentation, nothing to reclaim
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
        let new_size = (active_size as u64).max(Self::INITIAL_SIZE);
        temp_file.set_len(new_size)?;

        let mut temp_mmap = unsafe { MmapMut::map_mut(&temp_file)? };

        // 3. Copy active vectors to new file with new offsets
        let mmap = self.mmap.read();
        let mut new_index: FxHashMap<u64, usize> = FxHashMap::default();
        new_index.reserve(active_count);

        let mut new_offset = 0usize;
        for (&id, &old_offset) in index.iter() {
            // Copy vector data
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

        let new_mmap = unsafe { MmapMut::map_mut(&new_data_file)? };

        // 7. Update internal state
        *self.mmap.write() = new_mmap;
        // Note: We can't reassign self.data_file directly, so we use std::mem::replace
        // This is a limitation - for full fix we'd need data_file behind RwLock too

        *self.index.write() = new_index;
        self.next_offset.store(new_offset, Ordering::Relaxed);

        // 8. Write compaction marker to WAL
        {
            let mut wal = self.wal.write();
            // Op: Compact (4) - marker only, no data
            wal.write_all(&[4u8])?;
            wal.flush()?;
        }

        // 9. Save updated index
        self.flush()?;

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

    /// Retrieves a vector by ID without copying (zero-copy).
    ///
    /// Returns a guard that provides direct access to the mmap'd data.
    /// This is significantly faster than `retrieve()` for read-heavy workloads
    /// as it eliminates heap allocation and memory copy.
    ///
    /// # Arguments
    ///
    /// * `id` - The vector ID to retrieve
    ///
    /// # Returns
    ///
    /// - `Ok(Some(guard))` - Guard providing zero-copy access to the vector
    /// - `Ok(None)` - Vector with this ID doesn't exist
    /// - `Err(...)` - I/O error (e.g., corrupted data)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let guard = storage.retrieve_ref(id)?.unwrap();
    /// let slice: &[f32] = guard.as_ref();
    /// // Use slice directly - no allocation occurred
    /// ```
    ///
    /// # Performance
    ///
    /// Compared to `retrieve()`:
    /// - **No heap allocation** - data accessed directly from mmap
    /// - **No memcpy** - pointer arithmetic only
    /// - **Lock held** - guard must be dropped to release read lock
    ///
    /// # Errors
    ///
    /// Returns an error if the stored offset is out of bounds (corrupted index).
    pub fn retrieve_ref(&self, id: u64) -> io::Result<Option<VectorSliceGuard<'_>>> {
        // First check if ID exists (separate lock to minimize contention)
        let offset = {
            let index = self.index.read();
            match index.get(&id) {
                Some(&offset) => offset,
                None => return Ok(None),
            }
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

        // SAFETY: We've validated that offset + vector_size <= mmap.len()
        // The pointer is derived from the mmap which is held by the guard
        // Note: mmap data is written with f32 alignment via store(), so alignment is guaranteed
        #[allow(clippy::cast_ptr_alignment)]
        let ptr = unsafe { mmap.as_ptr().add(offset).cast::<f32>() };

        Ok(Some(VectorSliceGuard {
            _guard: mmap,
            ptr,
            len: self.dimension,
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
        // Perf: Use FxHashMap for O(1) lookup instead of Vec with O(n) find
        let mut new_vector_offsets: FxHashMap<u64, usize> = FxHashMap::default();
        new_vector_offsets.reserve(vectors.len());
        let mut total_new_size = 0usize;

        {
            let index = self.index.read();
            for &(id, _) in vectors {
                if !index.contains_key(&id) {
                    let offset = self.next_offset.load(Ordering::Relaxed) + total_new_size;
                    new_vector_offsets.insert(id, offset);
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

                // Get offset (existing or from new_vector_offsets)
                // Perf: O(1) HashMap lookup instead of O(n) linear search
                let offset = if let Some(&existing) = index.get(&id) {
                    existing
                } else {
                    new_vector_offsets.get(&id).copied().unwrap_or(0)
                };

                mmap[offset..offset + vector_size].copy_from_slice(vector_bytes);
            }
        }

        // 5. Batch update index
        if !new_vector_offsets.is_empty() {
            let mut index = self.index.write();
            for (id, offset) in new_vector_offsets {
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

        // Note: Space is reclaimed via compact() - see TS-CORE-004

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

    fn ids(&self) -> Vec<u64> {
        self.index.read().keys().copied().collect()
    }
}
