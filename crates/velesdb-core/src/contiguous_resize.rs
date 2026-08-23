//! Resize and reallocation logic for [`ContiguousVectors`].
//!
//! Extracted from [`super::perf_optimizations`] to isolate the allocation-growth
//! concern from the core storage API. Uses [`AllocGuard`] for panic-safe buffer
//! migration during capacity changes.
//!
//! [`AllocGuard`]: crate::alloc_guard::AllocGuard

use std::alloc::dealloc;
use std::ptr::{self, NonNull};

use super::perf_optimizations::ContiguousVectors;

impl ContiguousVectors {
    /// Ensures the storage has capacity for at least `required_capacity` vectors.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AllocationFailed`] if reallocation fails.
    ///
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn ensure_capacity(&mut self, required_capacity: usize) -> crate::error::Result<()> {
        if required_capacity > self.capacity {
            // #899: `self.capacity * 2` could overflow `usize` and wrap to a tiny
            // value, defeating the `.max(required_capacity)` guard. Saturate the
            // doubling so growth always satisfies `required_capacity` and the
            // final layout-size check (in `resize`) still rejects true overflow.
            let doubled = self.capacity.saturating_mul(2);
            let new_capacity = required_capacity.max(doubled);
            self.resize(new_capacity)?;
        }
        Ok(())
    }

    /// Pre-allocates capacity for `additional` more vectors beyond the current length.
    ///
    /// Analogous to [`Vec::reserve`]: ensures the buffer can hold
    /// `self.len() + additional` vectors without reallocating. No-op if
    /// sufficient capacity already exists.
    ///
    /// Call before a batch push to guarantee `push_batch` won't resize.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AllocationFailed`] if reallocation fails.
    ///
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn reserve_additional(&mut self, additional: usize) -> crate::error::Result<()> {
        let required = self.count.saturating_add(additional);
        self.ensure_capacity(required)
    }

    /// Resizes the internal buffer.
    ///
    /// # P2 Audit + PERF-002: Panic-Safety with RAII Guard
    ///
    /// This function uses `AllocGuard` for panic-safe allocation:
    /// 1. New buffer is allocated via RAII guard (auto-freed on panic)
    /// 2. Data is copied to new buffer
    /// 3. Guard ownership is transferred (no auto-free)
    /// 4. Old buffer is deallocated
    /// 5. State is updated atomically
    ///
    /// If panic occurs during copy, the guard ensures new buffer is freed.
    pub(crate) fn resize(&mut self, new_capacity: usize) -> crate::error::Result<()> {
        if new_capacity <= self.capacity {
            return Ok(());
        }

        if !self.backing.is_heap() {
            return self.resize_file_backed(new_capacity);
        }

        let old_layout = Self::layout(self.dimension, self.capacity)?;
        let new_layout = Self::layout(self.dimension, new_capacity)?;

        let new_data = Self::alloc_and_copy(new_layout, self.data, self.count, self.dimension)?;

        // Deallocate old buffer
        // SAFETY: self.data was allocated with old_layout, is non-null (NonNull invariant)
        // - Condition 1: old_layout matches the allocation parameters.
        // - Condition 2: Pointer is non-null per NonNull invariant.
        // SAFETY: Free old buffer after data migration to new buffer.
        unsafe {
            dealloc(self.data.as_ptr().cast::<u8>(), old_layout);
        }

        // Update state (all-or-nothing)
        self.data = new_data;
        self.capacity = new_capacity;
        Ok(())
    }

    /// Grows a file-backed arena by extending and re-mapping its file.
    ///
    /// No copy happens: the bytes already written keep their place on disk and
    /// the kernel hands back a mapping that covers more of the same file. The
    /// heap path cannot do this — it must `memcpy` the whole arena into a
    /// larger block — so growth is strictly cheaper here.
    ///
    /// The freshly appended range reads as zeros, matching the `alloc_zeroed`
    /// guarantee `insert_at` depends on when it leaves gaps.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AllocationFailed`] if the file cannot be extended or
    /// re-mapped. The arena is left untouched and usable in that case.
    ///
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    #[cfg(feature = "persistence")]
    fn resize_file_backed(&mut self, new_capacity: usize) -> crate::error::Result<()> {
        use crate::perf_optimizations::ArenaBacking;

        let byte_len = Self::byte_size(self.dimension, new_capacity)?;
        let ArenaBacking::FileMapped(ref mut arena) = self.backing else {
            return Err(crate::error::Error::Internal(
                "resize_file_backed called on a heap-backed arena".to_string(),
            ));
        };
        arena.grow(byte_len).map_err(|e| {
            crate::error::Error::AllocationFailed(format!(
                "failed to grow the file-backed vector arena to {byte_len} bytes: {e}"
            ))
        })?;
        // Re-derive the pointer: growing re-maps, so the old address is stale.
        let data = arena.data_ptr().map_err(|e| {
            crate::error::Error::AllocationFailed(format!(
                "grown vector arena produced no usable data pointer: {e}"
            ))
        })?;
        self.data = data;
        self.capacity = new_capacity;
        Ok(())
    }

    /// Without `persistence` there is no file-backed variant to grow.
    ///
    /// [`ArenaBacking::Heap`] is then the only constructible backing, so this
    /// is unreachable rather than merely unused — it exists to keep `resize`
    /// free of `cfg` noise.
    ///
    /// [`ArenaBacking::Heap`]: crate::perf_optimizations::ArenaBacking::Heap
    ///
    /// # Errors
    ///
    /// Always returns [`Error::Internal`].
    ///
    /// [`Error::Internal`]: crate::error::Error::Internal
    #[cfg(not(feature = "persistence"))]
    #[allow(clippy::unnecessary_wraps)]
    fn resize_file_backed(&mut self, _new_capacity: usize) -> crate::error::Result<()> {
        Err(crate::error::Error::Internal(
            "file-backed vector arenas require the persistence feature".to_string(),
        ))
    }

    /// Allocates a new buffer and copies existing data into it.
    ///
    /// Uses `AllocGuard` for panic-safety: if copy panics, the guard drops
    /// and frees the new buffer automatically.
    #[allow(clippy::cast_ptr_alignment)] // Layout is 64-byte aligned
    fn alloc_and_copy(
        new_layout: std::alloc::Layout,
        src: NonNull<f32>,
        count: usize,
        dimension: usize,
    ) -> crate::error::Result<NonNull<f32>> {
        use crate::alloc_guard::AllocGuard;

        // Allocate zero-initialized buffer with RAII guard (PERF-002)
        let guard = AllocGuard::new_zeroed(new_layout).ok_or_else(|| {
            crate::error::Error::AllocationFailed(format!(
                "Failed to allocate {} bytes for ContiguousVectors resize",
                new_layout.size()
            ))
        })?;

        // EPIC-032/US-002: Use NonNull for type-level guarantee
        let new_data = NonNull::new(guard.cast::<f32>()).ok_or_else(|| {
            crate::error::Error::AllocationFailed("AllocGuard returned null pointer".to_string())
        })?;

        // Copy existing data to new buffer
        if count > 0 {
            let copy_size = count * dimension;
            // SAFETY: Both pointers are valid (NonNull), non-overlapping, and properly aligned
            // - Condition 1: Source pointer (src) is valid and properly aligned.
            // - Condition 2: Destination pointer (new_data) is valid and properly aligned.
            // - Condition 3: Pointers are non-overlapping (old and new allocations are distinct).
            // SAFETY: Migrate data to newly allocated buffer during resize.
            unsafe {
                ptr::copy_nonoverlapping(src.as_ptr(), new_data.as_ptr(), copy_size);
            }
        }

        // Transfer ownership - guard won't free on drop anymore
        let _ = guard.into_raw();

        Ok(new_data)
    }
}
