//! RAII guards for safe manual memory management.
//!
//! # PERF-002: Allocation Guard
//!
//! Provides panic-safe allocation patterns for code that must use
//! manual memory management (e.g., cache-aligned buffers).
//!
//! # Usage
//!
//! ```rust,ignore
//! use velesdb_core::alloc_guard::AllocGuard;
//! use std::alloc::Layout;
//!
//! let layout = Layout::from_size_align(1024, 64).unwrap();
//! let guard = AllocGuard::new(layout)?;
//!
//! // Use guard.as_ptr() for operations...
//! // If panic occurs, memory is automatically freed
//!
//! // Transfer ownership when done
//! let ptr = guard.into_raw();
//! ```

use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;

/// RAII guard for raw allocations.
///
/// Ensures memory is deallocated if dropped, preventing leaks on panic.
/// Use `into_raw()` to take ownership and prevent deallocation.
#[derive(Debug)]
pub struct AllocGuard {
    ptr: NonNull<u8>,
    layout: Layout,
    /// If true, memory will be deallocated on drop
    owns_memory: bool,
}

impl AllocGuard {
    /// Allocates memory with the given layout.
    ///
    /// # Returns
    ///
    /// - `Some(guard)` if allocation succeeded
    /// - `None` if allocation failed (OOM) or layout size is zero
    ///
    /// # Panics
    ///
    /// This method does not panic. However, callers typically use
    /// `unwrap_or_else(|| panic!(...))` which will panic on OOM.
    ///
    /// # Safety
    ///
    /// The returned guard manages raw memory. The caller must ensure
    /// proper initialization before use.
    #[must_use]
    pub fn new(layout: Layout) -> Option<Self> {
        if layout.size() == 0 {
            return None;
        }

        // SAFETY: Layout is valid (non-zero size)
        let ptr = unsafe { alloc(layout) };

        NonNull::new(ptr).map(|ptr| Self {
            ptr,
            layout,
            owns_memory: true,
        })
    }

    /// Returns the raw pointer to the allocated memory.
    #[inline]
    #[must_use]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Returns the layout used for this allocation.
    #[inline]
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Transfers ownership of the memory, preventing deallocation on drop.
    ///
    /// # Returns
    ///
    /// The raw pointer to the allocated memory. The caller is now
    /// responsible for deallocating it with the same layout.
    #[inline]
    #[must_use]
    pub fn into_raw(mut self) -> *mut u8 {
        self.owns_memory = false;
        self.ptr.as_ptr()
    }

    /// Casts the pointer to a specific type.
    ///
    /// # Safety
    ///
    /// The caller must ensure the layout is compatible with type T.
    #[inline]
    #[must_use]
    pub fn cast<T>(&self) -> *mut T {
        self.ptr.as_ptr().cast()
    }
}

impl Drop for AllocGuard {
    fn drop(&mut self) {
        if self.owns_memory {
            // SAFETY: ptr was allocated with self.layout and we own it
            unsafe {
                dealloc(self.ptr.as_ptr(), self.layout);
            }
        }
    }
}

// AllocGuard is Send if the underlying memory can be sent between threads
// SAFETY: Raw memory has no thread affinity
unsafe impl Send for AllocGuard {}

// AllocGuard is NOT Sync - concurrent access to raw memory is unsafe
// (intentionally not implementing Sync)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_guard_basic() {
        let layout = Layout::from_size_align(1024, 8).unwrap();
        let guard = AllocGuard::new(layout).expect("allocation failed");

        assert!(!guard.as_ptr().is_null());
        assert_eq!(guard.layout().size(), 1024);
        assert_eq!(guard.layout().align(), 8);
    }

    #[test]
    fn test_alloc_guard_into_raw() {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let guard = AllocGuard::new(layout).expect("allocation failed");
        let ptr = guard.into_raw();

        // Must manually deallocate
        assert!(!ptr.is_null());
        unsafe {
            dealloc(ptr, layout);
        }
    }

    #[test]
    fn test_alloc_guard_zero_size() {
        let layout = Layout::from_size_align(0, 1).unwrap();
        assert!(AllocGuard::new(layout).is_none());
    }

    #[test]
    fn test_alloc_guard_aligned() {
        // Cache-line aligned (64 bytes)
        let layout = Layout::from_size_align(256, 64).unwrap();
        let guard = AllocGuard::new(layout).expect("allocation failed");

        let addr = guard.as_ptr() as usize;
        assert_eq!(addr % 64, 0, "Not cache-line aligned");
    }

    #[test]
    fn test_alloc_guard_cast() {
        let layout =
            Layout::from_size_align(std::mem::size_of::<f32>() * 10, std::mem::align_of::<f32>())
                .unwrap();

        let guard = AllocGuard::new(layout).expect("allocation failed");
        let float_ptr: *mut f32 = guard.cast();

        // Write some data
        #[allow(clippy::cast_precision_loss)]
        unsafe {
            for i in 0..10 {
                *float_ptr.add(i) = i as f32;
            }
        }

        // Read back
        #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
        unsafe {
            for i in 0..10 {
                assert_eq!(*float_ptr.add(i), i as f32);
            }
        }
    }

    #[test]
    fn test_alloc_guard_drop_frees_memory() {
        // This test verifies the guard deallocates on drop
        // We can't directly verify deallocation, but we can ensure no panic
        for _ in 0..1000 {
            let layout = Layout::from_size_align(1024, 8).unwrap();
            let _guard = AllocGuard::new(layout);
            // guard dropped here, memory freed
        }
    }

    #[test]
    fn test_alloc_guard_panic_safety() {
        use std::panic;

        let layout = Layout::from_size_align(1024, 8).unwrap();

        // Simulate panic during operation
        let result = panic::catch_unwind(|| {
            let _guard = AllocGuard::new(layout).expect("allocation failed");
            panic!("simulated panic");
        });

        assert!(result.is_err());
        // Memory should have been freed by drop during unwind
    }
}
