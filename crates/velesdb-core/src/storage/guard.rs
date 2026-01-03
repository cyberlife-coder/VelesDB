//! Zero-copy guard for vector data from mmap storage.

use memmap2::MmapMut;
use parking_lot::RwLockReadGuard;

/// Zero-copy guard for vector data from mmap storage.
///
/// This guard holds a read lock on the mmap and provides direct access
/// to the vector data without any memory allocation or copy.
///
/// # Performance
///
/// Using `VectorSliceGuard` instead of `retrieve()` eliminates:
/// - Heap allocation for the result `Vec<f32>`
/// - Memory copy from mmap to the new vector
///
/// # Example
///
/// ```rust,ignore
/// let guard = storage.retrieve_ref(id)?.unwrap();
/// let slice: &[f32] = guard.as_ref();
/// // Use slice directly - no allocation occurred
/// ```
pub struct VectorSliceGuard<'a> {
    /// Read guard holding the mmap lock
    pub(super) _guard: RwLockReadGuard<'a, MmapMut>,
    /// Pointer to the start of vector data
    pub(super) ptr: *const f32,
    /// Number of f32 elements
    pub(super) len: usize,
}

// SAFETY: VectorSliceGuard is Send+Sync because:
// 1. The underlying data is in a memory-mapped file (shared memory)
// 2. We hold a RwLockReadGuard which ensures exclusive read access
// 3. The pointer is derived from the guard and valid for its lifetime
unsafe impl Send for VectorSliceGuard<'_> {}
unsafe impl Sync for VectorSliceGuard<'_> {}

impl VectorSliceGuard<'_> {
    /// Returns the vector data as a slice.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        // SAFETY: ptr and len were validated during construction,
        // and the guard ensures the mmap remains valid
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl AsRef<[f32]> for VectorSliceGuard<'_> {
    #[inline]
    fn as_ref(&self) -> &[f32] {
        self.as_slice()
    }
}

impl std::ops::Deref for VectorSliceGuard<'_> {
    type Target = [f32];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
