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

mod vector_io;
mod wal_replay;

use super::compaction;
use super::guard::VectorSliceGuard;
use super::log_payload::DurabilityMode;
use super::metrics::StorageMetrics;
use super::sharded_index::ShardedIndex;
use super::traits::VectorStorage;
use crate::metrics::global_guardrails_metrics;

use memmap2::MmapMut;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::error;

/// Memory-mapped file storage for vectors.
///
/// Uses a combination of an index file (ID -> offset) and a data file (raw vectors).
/// Also implements a simple WAL for durability.
#[allow(clippy::module_name_repetitions)]
pub struct MmapStorage {
    /// Directory path for storage files
    pub(super) path: PathBuf,
    /// Vector dimension
    pub(super) dimension: usize,
    /// In-memory index of ID -> file offset
    /// EPIC-033/US-004: Sharded for reduced lock contention on read-heavy workloads
    pub(super) index: ShardedIndex,
    /// Write-Ahead Log writer
    pub(super) wal: RwLock<io::BufWriter<File>>,
    /// File handle for the data file (kept open for resizing)
    pub(super) data_file: File,
    /// Memory mapped data file
    pub(super) mmap: RwLock<MmapMut>,
    /// Next available offset in the data file
    pub(super) next_offset: AtomicUsize,
    /// P0 Audit: Metrics for monitoring `ensure_capacity` latency
    pub(super) metrics: Arc<StorageMetrics>,
    /// Epoch counter incremented every time the mmap is remapped.
    ///
    /// # Overflow Safety
    ///
    /// Uses wrapping arithmetic (guaranteed by `fetch_add`). Even at 1 billion
    /// remaps/second, overflow would take ~584 years. The worst-case scenario
    /// on wrap is a false-positive panic in `VectorSliceGuard::as_slice()`,
    /// which is acceptable given the astronomical time required.
    pub(super) remap_epoch: AtomicU64,
    /// Controls WAL write and sync behavior for vector storage.
    ///
    /// Issue #423 Component 4: `DurabilityMode::None` skips WAL writes
    /// entirely for bulk import scenarios where data can be re-derived.
    /// Default is `Fsync` (unchanged from pre-#423 behavior).
    pub(super) durability: DurabilityMode,
}

impl MmapStorage {
    /// P2: Increased from 64KB to 16MB for better initial capacity.
    pub(super) const INITIAL_SIZE: u64 = 16 * 1024 * 1024;

    /// P2: Increased from 1MB to 64MB minimum growth.
    pub(super) const MIN_GROWTH: u64 = 64 * 1024 * 1024;

    /// P2: Growth factor for exponential pre-allocation.
    pub(super) const GROWTH_FACTOR: u64 = 2;

    /// Creates a new `MmapStorage` or opens an existing one.
    ///
    /// Uses the default durability mode (`Fsync`).
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
        Self::new_with_durability(path, dimension, DurabilityMode::default())
    }

    /// Creates a new `MmapStorage` with the specified durability mode.
    ///
    /// See [`DurabilityMode`] for available modes and their trade-offs.
    ///
    /// Issue #423 Component 4: `DurabilityMode::None` skips WAL writes
    /// entirely for bulk import scenarios. Data is written directly to
    /// the mmap file and is readable immediately, but not recoverable
    /// from WAL after a crash.
    ///
    /// # Arguments
    ///
    /// * `path` - Directory to store data
    /// * `dimension` - Vector dimension
    /// * `durability` - WAL write/sync behavior
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn new_with_durability<P: AsRef<Path>>(
        path: P,
        dimension: usize,
        durability: DurabilityMode,
    ) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;

        let data_path = path.join("vectors.dat");
        compaction::recover_compaction_artifacts(&data_path)?;

        let data_file = Self::open_data_file(&data_path)?;
        let mmap = Self::create_initial_mmap(&data_file)?;

        let wal_path = path.join("vectors.wal");
        let wal = Self::open_wal(&wal_path)?;

        let index_path = path.join("vectors.idx");
        let (index, next_offset) = Self::load_index(&index_path, dimension)?;

        let (mmap, next_offset) =
            Self::replay_wal(mmap, next_offset, &wal_path, &index, dimension)?;

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
            durability,
        })
    }

    /// Returns the current durability mode.
    #[must_use]
    pub fn durability(&self) -> DurabilityMode {
        self.durability
    }

    /// Sets the durability mode at runtime.
    pub fn set_durability_mode(&mut self, mode: DurabilityMode) {
        self.durability = mode;
    }

    /// Returns a reference to the storage metrics.
    #[must_use]
    pub fn metrics(&self) -> &StorageMetrics {
        &self.metrics
    }

    /// Opens or creates the data file, ensuring it has at least `INITIAL_SIZE` bytes.
    fn open_data_file(data_path: &Path) -> io::Result<File> {
        let data_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data_path)?;

        let file_len = data_file.metadata()?.len();
        if file_len == 0 {
            data_file.set_len(Self::INITIAL_SIZE)?;
        }
        Ok(data_file)
    }

    /// Creates the initial memory map for the data file.
    fn create_initial_mmap(data_file: &File) -> io::Result<MmapMut> {
        // SAFETY: data_file is a valid, open file with set_len() called to ensure
        // the mapping range is fully allocated.
        // - Condition 1: File was opened with read+write permissions.
        // - Condition 2: set_len() was called to ensure the file has INITIAL_SIZE bytes.
        // - Condition 3: MmapMut requires readable and writable file, guaranteed by OpenOptions.
        // SAFETY: Memory mapping requires unsafe due to potential for undefined behavior if file is truncated externally.
        unsafe { MmapMut::map_mut(data_file) }
    }

    /// Opens or creates the WAL file wrapped in a buffered writer.
    fn open_wal(wal_path: &Path) -> io::Result<io::BufWriter<File>> {
        let wal_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(wal_path)?;
        Ok(io::BufWriter::new(wal_file))
    }

    /// Loads the sharded index from disk, returning the index and the next write offset.
    fn load_index(index_path: &Path, dimension: usize) -> io::Result<(ShardedIndex, usize)> {
        if !index_path.exists() {
            return Ok((ShardedIndex::new(), 0));
        }

        let bytes = std::fs::read(index_path)?;
        let flat_index: FxHashMap<u64, usize> = postcard::from_bytes(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let max_offset = flat_index.values().max().copied().unwrap_or(0);
        let size = if flat_index.is_empty() {
            0
        } else {
            max_offset + dimension * 4
        };

        Ok((ShardedIndex::from_hashmap(flat_index), size))
    }

    /// Replays the WAL to recover writes since the last flush.
    fn replay_wal(
        mut mmap: MmapMut,
        mut next_offset: usize,
        wal_path: &Path,
        index: &ShardedIndex,
        dimension: usize,
    ) -> io::Result<(MmapMut, usize)> {
        let replayed = wal_replay::replay_wal_to_index(
            wal_path,
            index,
            dimension,
            &mut mmap,
            &mut next_offset,
        )?;
        if replayed > 0 {
            mmap.flush()?;
        }
        Ok((mmap, next_offset))
    }

    // ensure_capacity, reserve_capacity, compact, fragmentation_ratio are in mmap_capacity.rs

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
        let Some(offset) = self.index.get(id) else {
            return Ok(None);
        };

        // Now acquire mmap read lock and validate bounds
        let mmap = self.mmap.read();
        let vector_size = self.dimension * std::mem::size_of::<f32>();

        Self::validate_offset(offset, vector_size, mmap.len())?;

        #[allow(clippy::cast_ptr_alignment)]
        // SAFETY: We validated bounds/alignment above and keep the mmap read lock
        // in `VectorSliceGuard`, so `ptr` stays valid for the guard lifetime.
        // - Condition 1: `end <= mmap.len()` guarantees the addressed range exists.
        // - Condition 2: `offset` is aligned to `align_of::<f32>()`.
        // - Condition 3: `mmap` read lock pins the mapping while guard is alive.
        // SAFETY: Zero-copy read path needs raw pointer conversion to `[f32]`.
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

    /// Validates that `offset` is within bounds and f32-aligned.
    ///
    /// Returns an error if the offset overflows, is out of bounds, or
    /// is not aligned to `align_of::<f32>()`.
    fn validate_offset(offset: usize, vector_size: usize, mmap_len: usize) -> io::Result<()> {
        let end = offset.checked_add(vector_size).ok_or_else(|| {
            global_guardrails_metrics().record_invalid_offset_read_error();
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Offset arithmetic overflow while reading vector",
            )
        })?;

        if end > mmap_len {
            global_guardrails_metrics().record_invalid_offset_read_error();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Offset out of bounds",
            ));
        }

        // EPIC-032/US-001: Verify alignment before pointer cast
        if offset % std::mem::align_of::<f32>() != 0 {
            global_guardrails_metrics().record_invalid_offset_read_error();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "EPIC-032/US-001: offset {offset} is not f32-aligned (must be multiple of {})",
                    std::mem::align_of::<f32>()
                ),
            ));
        }

        Ok(())
    }

    /// Persists the `vectors.idx` index file to disk with fsync.
    ///
    /// Issue #423: Extracted from the former `flush()` to allow callers to
    /// control when the (expensive) index serialization happens. The WAL
    /// provides crash recovery even if this file is stale, so it can be
    /// deferred to compaction or explicit shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or I/O fails.
    pub fn flush_index(&self) -> io::Result<()> {
        // EPIC-033/US-004: Convert ShardedIndex to flat HashMap for serialization
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

    /// Full durability flush: WAL + mmap + `vectors.idx`.
    ///
    /// Equivalent to the pre-#423 `flush()` behavior. Use this on shutdown
    /// or before compaction to ensure the index file is up-to-date, avoiding
    /// a full WAL replay on the next startup.
    ///
    /// # Errors
    ///
    /// Returns an error if any I/O operation fails.
    pub fn flush_full(&mut self) -> io::Result<()> {
        self.flush()?;
        self.flush_index()
    }

    /// Attempts a best-effort durability sync during shutdown.
    ///
    /// This method never returns an error and never blocks on lock contention:
    /// if the WAL/mmap lock cannot be acquired immediately, the flush step is
    /// skipped and shutdown continues.
    ///
    /// Use explicit [`VectorStorage::flush`](crate::storage::traits::VectorStorage::flush)
    /// to obtain a deterministic durability barrier.
    pub(crate) fn flush_on_shutdown_best_effort(&self) {
        // 1. Flush WAL first (operation log)
        self.try_flush_wal();

        // 2. Flush mmap to persist vector bytes
        self.try_flush_mmap();
    }

    /// Best-effort WAL flush: skips if lock is contended.
    fn try_flush_wal(&self) {
        if let Some(mut wal) = self.wal.try_write() {
            if let Err(e) = wal.flush() {
                error!(?e, "Failed to flush WAL in MmapStorage shutdown path");
            }
            if let Err(e) = wal.get_ref().sync_all() {
                error!(?e, "Failed to fsync WAL in MmapStorage shutdown path");
            }
        }
    }

    /// Best-effort mmap flush: skips if lock is contended.
    fn try_flush_mmap(&self) {
        if let Some(mmap) = self.mmap.try_write() {
            if let Err(e) = mmap.flush() {
                error!(?e, "Failed to flush mmap in MmapStorage shutdown path");
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Drop implementation – best-effort sync on graceful shutdown.
//
// Important: `drop` is not a transactional durability boundary.
// Call `flush()` explicitly when the caller requires deterministic durability.
// -----------------------------------------------------------------------------
impl Drop for MmapStorage {
    fn drop(&mut self) {
        self.flush_on_shutdown_best_effort();
    }
}
