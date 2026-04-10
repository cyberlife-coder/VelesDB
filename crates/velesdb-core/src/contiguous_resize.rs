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
            let new_capacity = required_capacity.max(self.capacity * 2);
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
