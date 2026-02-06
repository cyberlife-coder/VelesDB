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
/// ```rust,no_run
/// # use velesdb_core::storage::{MmapStorage, VectorSliceGuard};
/// # use std::io;
/// # fn example() -> io::Result<()> {
/// # let mut storage = MmapStorage::new("/tmp/test", 128)?;
/// # let id = 1u64;
/// // Get zero-copy access to a vector
/// let guard: Option<VectorSliceGuard> = storage.retrieve_ref(id)?;
/// if let Some(guard) = guard {
///     let slice: &[f32] = guard.as_ref();
///     // Use slice directly - no allocation occurred
/// }
/// # Ok(())
/// # }
/// ```
use std::sync::atomic::AtomicU64;

/// Zero-copy guard for vector data from mmap storage.
/// Holds a read-lock on the mmap and validates that the underlying mapping
/// hasn't been remapped via an *epoch* counter.
///
/// # Epoch Validation
///
/// The guard captures the epoch at creation and validates it on each access.
/// If the mmap was remapped (epoch changed), access panics to prevent UB.
///
/// The epoch uses wrapping `u64` arithmetic. Overflow is theoretically possible
/// after 2^64 remaps (~584 years at 1B/sec) but practically irrelevant.
pub struct VectorSliceGuard<'a> {
    /// Read guard holding the mmap lock – guarantees the mapping is pinned for the guard lifetime
    pub(super) _guard: RwLockReadGuard<'a, MmapMut>,
    /// Pointer to the start of vector data
    pub(super) ptr: *const f32,
    /// Number of f32 elements
    pub(super) len: usize,
    /// Pointer to the global epoch counter inside `MmapStorage`
    pub(super) epoch_ptr: &'a AtomicU64,
    /// Epoch captured at construction
    pub(super) epoch_at_creation: u64,
}

// SAFETY: VectorSliceGuard is Send+Sync because:
// 1. The underlying data is in a memory-mapped file (shared memory)
// 2. We hold a RwLockReadGuard which ensures exclusive read access
// 3. The pointer is derived from the guard and valid for its lifetime
// SAFETY: The guard enforces:
// * Lifetime tied to `_guard` (RwLockReadGuard) ⇒ mapping is pinned
// * Epoch check prevents access after remap
// The data is read-only, therefore Send + Sync are sound.
// Reason: Raw pointer is derived from MmapMut which is Send+Sync
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for VectorSliceGuard<'_> {}
// Reason: Raw pointer is derived from MmapMut which is Send+Sync
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Sync for VectorSliceGuard<'_> {}

impl VectorSliceGuard<'_> {
    /// Returns the vector data as a slice.
    ///
    /// # Panics
    ///
    /// Panics if the underlying mmap has been remapped since this guard was created.
    /// This indicates a programming error where a guard outlived a resize operation.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        // SAFETY: ptr and len were validated during construction,
        // and the guard ensures the mmap remains valid
        // Verify epoch – if the mmap was remapped the pointer is invalid
        let current = self.epoch_ptr.load(std::sync::atomic::Ordering::Acquire);
        assert!(
            current == self.epoch_at_creation,
            "Mmap was remapped; VectorSliceGuard is invalid"
        );
        // SAFETY: epoch check guarantees the pointer still refers to the currently mapped region
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
