//! Performance optimizations module for ultra-fast vector operations.
//!
//! This module provides:
//! - **Contiguous vector storage**: Cache-friendly memory layout
//! - **Prefetch hints**: CPU cache warming for HNSW traversal
//! - **Batch distance computation**: SIMD-optimized batch operations
//!
//! # Performance Targets
//!
//! - Bulk import: 50K+ vectors/sec at 768D
//! - Search latency: < 1ms for 1M vectors
//! - Memory efficiency: 50% reduction with FP16
//!
//! # Safety (EPIC-032/US-002)
//!
//! `ContiguousVectors` uses `NonNull<f32>` to encode non-nullness at the type level,
//! eliminating null pointer checks and making invariants explicit. Memory is managed
//! via RAII with `AllocGuard` for panic-safe resize operations.

#[cfg(feature = "persistence")]
use crate::contiguous_file_arena::ExistingBytes;
use crate::validation::{validate_dimension, validate_dimension_match};
use std::alloc::{alloc_zeroed, Layout};
use std::fmt;
use std::ptr::{self, NonNull};

// =============================================================================
// Contiguous Vector Storage (Cache-Optimized)
// =============================================================================

/// Contiguous memory layout for vectors (cache-friendly).
///
/// Stores all vectors in a single contiguous buffer to maximize
/// cache locality and enable SIMD prefetching.
///
/// # Memory Layout
///
/// ```text
/// [v0_d0, v0_d1, ..., v0_dn, v1_d0, v1_d1, ..., v1_dn, ...]
/// ```
///
/// # Safety Invariants (EPIC-032/US-002)
///
/// - `data` is always non-null (enforced by `NonNull`)
/// - `data` points to memory allocated with 64-byte alignment
/// - `capacity * dimension * sizeof(f32)` bytes are always allocated
/// - `count <= capacity` is always maintained
pub struct ContiguousVectors {
    /// Non-null contiguous data buffer (EPIC-032/US-002: type-level non-null guarantee)
    pub(crate) data: NonNull<f32>,
    /// Vector dimension
    pub(crate) dimension: usize,
    /// Number of vectors stored
    pub(crate) count: usize,
    /// Allocated capacity (number of vectors)
    pub(crate) capacity: usize,
    /// Where `data` came from, and therefore how it must be released.
    pub(crate) backing: ArenaBacking,
}

/// How a [`ContiguousVectors`] buffer was obtained.
///
/// The variant decides who frees `data` — which is why it lives next to the
/// pointer rather than being inferred at each site. Getting this wrong means
/// calling `dealloc` on a mapping, so the enum is the invariant.
pub(crate) enum ArenaBacking {
    /// `alloc_zeroed` on the arena's [`Layout`], released with `dealloc`.
    ///
    /// The default, and the only backing available without `persistence`.
    Heap,
    /// A file mapping that owns the bytes; released by dropping the mapping.
    ///
    /// Chosen when the index wants its f32 arena evictable rather than
    /// resident — see [`crate::contiguous_file_arena`] for why (#2112).
    #[cfg(feature = "persistence")]
    FileMapped(Box<crate::contiguous_file_arena::FileArena>),
}

impl ArenaBacking {
    /// Whether this buffer must be released with `dealloc`.
    ///
    /// The single place that answers "is this pointer the allocator's?", so a
    /// new variant cannot silently fall into the deallocating branch.
    pub(crate) const fn is_heap(&self) -> bool {
        matches!(*self, Self::Heap)
    }
}

impl fmt::Debug for ArenaBacking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Heap => f.write_str("Heap"),
            #[cfg(feature = "persistence")]
            Self::FileMapped(ref arena) => write!(f, "FileMapped({:?})", arena.path()),
        }
    }
}

// SAFETY: `ContiguousVectors` is `Send` because it owns its buffer outright.
// - Condition 1: The backing buffer is uniquely owned by the struct. For
//   `ArenaBacking::Heap` the allocator establishes that. For `FileMapped` a
//   path is NOT unique on its own, so `FileArena` takes an exclusive advisory
//   lock before mapping and holds it for the mapping's whole life — that lock
//   is what makes this condition true rather than hoped for.
// - Condition 2: Mutation requires `&mut self` or lock-guarded interior access.
// - Condition 3: the mapped variant is `Send` on its own terms — `FileArena`
//   carries its own justified impl next to the mapping it describes, rather
//   than being covered by this one at a distance.
// SAFETY: Moving ownership of this container between threads is sound.
unsafe impl Send for ContiguousVectors {}
// SAFETY: `ContiguousVectors` is `Sync` because shared access is read-only.
// - Condition 1: All writes happen through methods requiring mutable or exclusive lock access.
// - Condition 2: Returned shared slices borrow immutably and cannot mutate internal state.
// - Condition 3: no second mapping of the same bytes can exist to write them
//   behind those shared references — see Condition 1 of the `Send` impl.
// SAFETY: Concurrent shared references cannot violate aliasing rules.
unsafe impl Sync for ContiguousVectors {}

impl fmt::Debug for ContiguousVectors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContiguousVectors")
            .field("dimension", &self.dimension)
            .field("count", &self.count)
            .field("capacity", &self.capacity)
            .field("backing", &self.backing)
            .finish_non_exhaustive()
    }
}

impl ContiguousVectors {
    /// Creates a new `ContiguousVectors` with the given dimension and initial capacity.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension (must be > 0)
    /// * `capacity` - Initial capacity (number of vectors)
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDimension`] if `dimension` is 0 or exceeds
    /// [`MAX_DIMENSION`](crate::validation::MAX_DIMENSION).
    /// Returns [`Error::AllocationFailed`] if memory allocation fails or exceeds
    /// the [`AllocGuard`](crate::alloc_guard::AllocGuard) ceiling.
    ///
    /// [`Error::InvalidDimension`]: crate::error::Error::InvalidDimension
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    #[allow(clippy::cast_ptr_alignment)] // Layout is 64-byte aligned
    pub fn new(dimension: usize, capacity: usize) -> crate::error::Result<Self> {
        // Enforce the advertised dimension range (#899): previously `max: 65_536`
        // was reported in errors but never validated, so an oversized dimension
        // could drive `dimension * capacity` products toward overflow.
        validate_dimension(dimension)?;

        let capacity = capacity.max(16); // Minimum 16 vectors

        // Reject pathological/attacker-sized allocations before touching the
        // allocator (#899). Legitimate large indexes stay well under the ceiling.
        crate::alloc_guard::check_alloc_bound(Self::byte_size(dimension, capacity)?)?;
        let layout = Self::layout(dimension, capacity)?;

        // SAFETY: `alloc_zeroed` requires a valid non-zero layout.
        // - Condition 1: `dimension > 0` and `capacity >= 16` guarantee non-zero size.
        // - Condition 2: `layout` is built via `Layout::from_size_align` and therefore valid.
        // SAFETY: Zero-initialized allocation guarantees all f32 slots are 0.0,
        // preventing UB when `insert_at` creates sparse gaps (indices 0..N not all written).
        let ptr = unsafe { alloc_zeroed(layout) };

        // EPIC-032/US-002: Use NonNull for type-level non-null guarantee
        let data = NonNull::new(ptr.cast::<f32>()).ok_or_else(|| {
            crate::error::Error::AllocationFailed(
                "ContiguousVectors: allocator returned null".to_string(),
            )
        })?;

        Ok(Self {
            data,
            dimension,
            count: 0,
            capacity,
            backing: ArenaBacking::Heap,
        })
    }

    /// Creates a `ContiguousVectors` whose buffer is a file mapping.
    ///
    /// The arena behaves identically to a heap-backed one — same layout, same
    /// accessors, same zero-fill guarantee — but its pages are evictable, so
    /// they do not count against the resident set when nothing is touching
    /// them. That is what lets a quantized index hold `codes + graph`
    /// resident and page the f32 in only to re-rank (#2112).
    ///
    /// `path` is created if absent, and whatever it held is discarded — under
    /// the arena's exclusive lock, never at open time. Use
    /// [`open_file_backed`](Self::open_file_backed) to adopt a file whose
    /// contents are already the arena.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDimension`] if `dimension` is out of range,
    /// [`Error::AllocationFailed`] if the size overflows or the ceiling
    /// rejects it, and [`Error::Io`] if the file cannot be created or mapped.
    ///
    /// [`Error::InvalidDimension`]: crate::error::Error::InvalidDimension
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    /// [`Error::Io`]: crate::error::Error::Io
    #[cfg(feature = "persistence")]
    pub fn new_file_backed(
        path: &std::path::Path,
        dimension: usize,
        capacity: usize,
    ) -> crate::error::Result<Self> {
        Self::file_backed(path, dimension, capacity, ExistingBytes::Discard)
    }

    /// Adopts an existing file as the arena without truncating it.
    ///
    /// The mapped bytes are taken as `count` already-written vectors, so this
    /// is the load path: no deserialization, no copy, just a mapping.
    ///
    /// # Errors
    ///
    /// As [`new_file_backed`](Self::new_file_backed), plus
    /// [`Error::Internal`] if `count` exceeds `capacity`.
    ///
    /// [`Error::Internal`]: crate::error::Error::Internal
    #[cfg(feature = "persistence")]
    pub fn open_file_backed(
        path: &std::path::Path,
        dimension: usize,
        capacity: usize,
        count: usize,
    ) -> crate::error::Result<Self> {
        if count > capacity {
            return Err(crate::error::Error::Internal(format!(
                "vector arena count {count} exceeds capacity {capacity}"
            )));
        }
        let mut storage = Self::file_backed(path, dimension, capacity, ExistingBytes::Keep)?;
        storage.count = count;
        Ok(storage)
    }

    /// Shared construction for the two file-backed entry points.
    #[cfg(feature = "persistence")]
    fn file_backed(
        path: &std::path::Path,
        dimension: usize,
        capacity: usize,
        existing: ExistingBytes,
    ) -> crate::error::Result<Self> {
        use crate::contiguous_file_arena::FileArena;

        validate_dimension(dimension)?;
        let capacity = capacity.max(16);
        let byte_len = Self::byte_size(dimension, capacity)?;
        crate::alloc_guard::check_alloc_bound(byte_len)?;

        let arena = match existing {
            ExistingBytes::Discard => FileArena::create(path, byte_len),
            ExistingBytes::Keep => FileArena::open(path, byte_len),
        }
        .map_err(|e| Self::arena_open_error(path, e))?;
        let data = arena.data_ptr();

        Ok(Self {
            data,
            dimension,
            count: 0,
            capacity,
            backing: ArenaBacking::FileMapped(Box::new(arena)),
        })
    }

    /// Classifies a failure to open an arena file.
    ///
    /// A refused exclusive lock means another holder has this arena, which is
    /// a different situation from a full disk or a missing directory and the
    /// caller can act on it differently. Collapsing both into
    /// [`Error::Io`] would throw that away, so the lock case is surfaced as
    /// the same locked-resource error `Database::open` uses for its data
    /// directory.
    ///
    /// [`Error::Io`]: crate::error::Error::Io
    #[cfg(feature = "persistence")]
    fn arena_open_error(path: &std::path::Path, error: std::io::Error) -> crate::error::Error {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            crate::error::Error::DatabaseLocked(path.display().to_string())
        } else {
            crate::error::Error::Io(error)
        }
    }

    /// Flushes a file-backed arena's dirty pages to disk.
    ///
    /// A no-op on a heap-backed arena, which has no file to reach.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the underlying `msync` fails.
    ///
    /// [`Error::Io`]: crate::error::Error::Io
    pub fn flush_backing(&self) -> crate::error::Result<()> {
        #[cfg(feature = "persistence")]
        if let ArenaBacking::FileMapped(ref arena) = self.backing {
            arena.flush().map_err(crate::error::Error::Io)?;
        }
        Ok(())
    }

    /// Returns the buffer size in bytes for `dimension * capacity` f32s.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AllocationFailed`] if `dimension * capacity * 4` overflows
    /// `usize`.
    ///
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub(crate) fn byte_size(dimension: usize, capacity: usize) -> crate::error::Result<usize> {
        dimension
            .checked_mul(capacity)
            .and_then(|s| s.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| {
                crate::error::Error::AllocationFailed(format!(
                    "Size overflow: {dimension} * {capacity} * {}",
                    std::mem::size_of::<f32>()
                ))
            })
    }

    /// Returns the memory layout for the given dimension and capacity.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AllocationFailed`] if the layout parameters are invalid.
    ///
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub(crate) fn layout(dimension: usize, capacity: usize) -> crate::error::Result<Layout> {
        let size = Self::byte_size(dimension, capacity)?;
        let align = 64; // Cache line alignment for optimal prefetch
        Layout::from_size_align(size.max(64), align)
            .map_err(|e| crate::error::Error::AllocationFailed(format!("Invalid layout: {e}")))
    }

    /// Returns the dimension of stored vectors.
    #[inline]
    #[must_use]
    pub const fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the number of vectors stored.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Returns true if no vectors are stored.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the capacity (max vectors before reallocation).
    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the raw contiguous buffer as a flat slice.
    ///
    /// The slice contains all vectors packed sequentially:
    /// `[v0_d0, v0_d1, ..., v1_d0, ...]`.
    /// Useful for GPU upload without copying.
    #[inline]
    #[must_use]
    pub fn as_flat_slice(&self) -> &[f32] {
        if self.count == 0 {
            return &[];
        }
        // `count <= capacity` and `capacity * dimension` was validated to fit in
        // `usize` at allocation time, so this product cannot overflow here.
        let total = self.count.saturating_mul(self.dimension);
        // SAFETY: All `capacity * dimension` f32s are valid because both initial allocation
        // (`alloc_zeroed`) and resize (`AllocGuard::new_zeroed`) zero-initialize the buffer.
        // `count * dimension <= capacity * dimension`, `data` is non-null per `NonNull`
        // invariant. Even sparse `insert_at` gaps contain valid 0.0 f32 values.
        // - Condition 1: `data` is a valid, aligned `NonNull<f32>` pointer.
        // - Condition 2: `total <= capacity * dimension` ensures the slice is within the allocation.
        // - Condition 3: All bytes in the allocation are initialized (zeroed or written).
        // SAFETY: Zero-copy GPU upload requires a contiguous &[f32] view.
        unsafe { std::slice::from_raw_parts(self.data.as_ptr(), total) }
    }

    /// Gathers vectors at the specified indices into a contiguous flat buffer.
    ///
    /// Returns a new `Vec<f32>` containing the selected vectors packed sequentially.
    /// Useful for GPU upload when only a subset of vectors is needed (e.g., reranking).
    ///
    /// # Important
    ///
    /// Out-of-bounds indices are silently skipped — the result may contain fewer
    /// vectors than `indices.len()`. Callers **must** validate
    /// `result.len() == indices.len() * dimension` before using the result in
    /// positional operations (e.g., `zip` with an ID map), otherwise scores
    /// will be misattributed to wrong IDs.
    #[must_use]
    pub fn gather_flat(&self, indices: &[usize]) -> Vec<f32> {
        // Saturating reservation hint: an overflowing `indices.len() * dimension`
        // would otherwise panic inside `Vec::with_capacity`. `extend_from_slice`
        // grows on demand, so a clamped hint stays correct (#899).
        let mut result = Vec::with_capacity(indices.len().saturating_mul(self.dimension));
        for &idx in indices {
            if let Some(vec) = self.get(idx) {
                result.extend_from_slice(vec);
            }
        }
        result
    }

    /// Returns total memory usage in bytes.
    ///
    /// Saturates instead of overflowing; `capacity * dimension * 4` was validated
    /// to fit in `usize` at allocation time, so saturation is unreachable for a
    /// live buffer and exists purely as a defensive backstop (#899).
    #[inline]
    #[must_use]
    pub const fn memory_bytes(&self) -> usize {
        self.capacity
            .saturating_mul(self.dimension)
            .saturating_mul(std::mem::size_of::<f32>())
    }

    /// Inserts a vector at a specific index.
    ///
    /// Automatically grows capacity if needed.
    /// Note: This allows sparse population. Uninitialized slots contain undefined
    /// data (or 0.0 if alloc gave zeroed memory).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if `vector.len() != self.dimension`.
    /// Returns [`Error::AllocationFailed`] if capacity growth fails.
    ///
    /// [`crate::error::Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn insert_at(&mut self, index: usize, vector: &[f32]) -> crate::error::Result<()> {
        validate_dimension_match(self.dimension, vector.len())?;

        // #899: `index == usize::MAX` made `index + 1` wrap to 0, so capacity was
        // never grown and `index * dimension` overflowed, producing an OOB
        // `copy_nonoverlapping` write in release builds. Reject overflow instead.
        let required = index.checked_add(1).ok_or_else(|| {
            crate::error::Error::AllocationFailed(format!(
                "insert_at: index {index} + 1 overflows usize"
            ))
        })?;
        self.ensure_capacity(required)?;

        let offset = index.checked_mul(self.dimension).ok_or_else(|| {
            crate::error::Error::AllocationFailed(format!(
                "insert_at: offset {index} * {} overflows usize",
                self.dimension
            ))
        })?;
        // SAFETY: We ensured capacity covers index, data is non-null (NonNull invariant)
        // - Condition 1: Capacity was verified to cover the target index.
        // - Condition 2: Both source and destination pointers are valid and properly aligned.
        // SAFETY: Efficient bulk memory copy for vector insertion.
        unsafe {
            ptr::copy_nonoverlapping(
                vector.as_ptr(),
                self.data.as_ptr().add(offset),
                self.dimension,
            );
        }

        // Update count if we're extending the "used" range.
        // `required == index + 1` (checked above), so this cannot overflow.
        if index >= self.count {
            self.count = required;
        }
        Ok(())
    }

    /// Adds a vector to the storage.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if `vector.len() != self.dimension`.
    /// Returns [`Error::AllocationFailed`] if capacity growth fails.
    ///
    /// [`crate::error::Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn push(&mut self, vector: &[f32]) -> crate::error::Result<()> {
        self.insert_at(self.count, vector)
    }

    /// Adds multiple vectors in batch (optimized).
    ///
    /// # Arguments
    ///
    /// * `vectors` - Iterator of vectors to add
    ///
    /// # Returns
    ///
    /// Number of vectors added.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] or [`Error::AllocationFailed`] on the
    /// first vector that fails. Vectors added before the failure remain in storage.
    ///
    /// [`crate::error::Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn push_batch(&mut self, vectors: &[&[f32]]) -> crate::error::Result<usize> {
        if vectors.is_empty() {
            return Ok(0);
        }
        // Validate all dimensions upfront to prevent partial writes on error.
        for vector in vectors {
            validate_dimension_match(self.dimension, vector.len())?;
        }
        let required = self.count.checked_add(vectors.len()).ok_or_else(|| {
            crate::error::Error::AllocationFailed(format!(
                "push_batch: count {} + {} overflows usize",
                self.count,
                vectors.len()
            ))
        })?;
        self.ensure_capacity(required)?;
        for vector in vectors {
            // `offset < required * dimension`, and `required * dimension` was
            // validated to fit in `usize` by `ensure_capacity`/`layout` above.
            let offset = self.count.saturating_mul(self.dimension);
            // SAFETY: ensure_capacity (called above) guarantees room for
            // self.count + vectors.len() elements, and all dimensions were
            // validated above so offset + dimension is within bounds.
            // - Condition 1: offset + dimension is within allocated buffer.
            // - Condition 2: Both pointers are valid and aligned for f32.
            // - Condition 3: &mut self guarantees exclusive access — no data race.
            // SAFETY: Batch push with single pre-allocation.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    vector.as_ptr(),
                    self.data.as_ptr().add(offset),
                    self.dimension,
                );
            }
            self.count += 1;
        }
        Ok(vectors.len())
    }

    /// Gets a vector by index.
    ///
    /// # Returns
    ///
    /// Slice to the vector data, or `None` if index is out of bounds.
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&[f32]> {
        if index >= self.count {
            // Note: In sparse mode, index < count doesn't guarantee it was initialized,
            // but for HNSW dense IDs it typically does.
            return None;
        }

        let offset = index * self.dimension;
        // SAFETY: Index is within bounds (checked against count, which is <= capacity)
        // - Condition 1: index < count ensures access is within initialized range.
        // - Condition 2: data is non-null per NonNull invariant.
        // SAFETY: Zero-copy slice creation from contiguous storage.
        Some(unsafe { std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension) })
    }

    /// Gets a mutable vector by index.
    ///
    /// Used by the HNSW batch-insert path to normalize cosine vectors
    /// in place after a raw `push_batch`, avoiding one owned intermediate
    /// buffer per vector (PERF2). Requires `&mut self`, so callers must
    /// hold the exclusive storage lock — no concurrent reader can observe
    /// a partially normalized vector.
    ///
    /// # Returns
    ///
    /// Mutable slice to the vector data, or `None` if index is out of bounds.
    #[inline]
    #[must_use]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut [f32]> {
        if index >= self.count {
            return None;
        }

        let offset = index * self.dimension;
        // SAFETY: Index is within bounds (checked against count, which is <= capacity)
        // - Condition 1: index < count ensures access is within initialized range.
        // - Condition 2: data is non-null per NonNull invariant.
        // - Condition 3: `&mut self` guarantees exclusive access — no aliasing.
        // SAFETY: In-place mutation of one vector slot in contiguous storage.
        Some(unsafe {
            std::slice::from_raw_parts_mut(self.data.as_ptr().add(offset), self.dimension)
        })
    }

    /// Gets a vector by index (unchecked).
    ///
    /// # Safety
    ///
    /// Caller must ensure `index < self.len()`.
    ///
    /// # Debug Assertions
    ///
    /// In debug builds, this function will panic if `index >= self.len()`.
    /// This catches bugs early during development without impacting release performance.
    #[inline]
    #[must_use]
    pub unsafe fn get_unchecked(&self, index: usize) -> &[f32] {
        debug_assert!(
            index < self.count,
            "index out of bounds: index={index}, count={}",
            self.count
        );
        let offset = index * self.dimension;
        // SAFETY: Caller guarantees index < count, data is non-null (NonNull invariant)
        // - Condition 1: Caller contract ensures index < count.
        // - Condition 2: data is non-null per NonNull invariant.
        // SAFETY: Performance-critical path requiring unchecked access.
        std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension)
    }

    /// Prefetches a vector into multiple cache levels for upcoming access.
    ///
    /// Uses cross-platform multi-cache-line prefetch (`x86_64` + `aarch64` + no-op fallback)
    /// to warm CPU caches before SIMD distance computation.
    #[inline]
    pub fn prefetch(&self, index: usize) {
        if index < self.count {
            let offset = index * self.dimension;
            // SAFETY: index < count implies offset is within allocated range,
            // data is non-null per NonNull invariant.
            // - Condition 1: Bounds check ensures offset + dimension <= capacity * dimension.
            // - Condition 2: NonNull guarantees pointer validity.
            // SAFETY: Create slice for cross-platform multi-cache-line prefetch.
            let vector = unsafe {
                std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension)
            };
            crate::simd_native::prefetch_vector_multi_cache_line(vector);
        }
    }
}

// Backward-compatible re-exports from contiguous_ops
pub use crate::contiguous_ops::{
    batch_cosine_similarities, batch_dot_products_simd, pad_to_simd_width,
};
