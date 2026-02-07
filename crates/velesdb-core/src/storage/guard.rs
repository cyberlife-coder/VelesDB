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

// SAFETY: `VectorSliceGuard` is `Send` because it carries read-only mapped data.
// - Condition 1: `_guard` pins the mapping and prevents concurrent remap mutation.
// - Condition 2: Epoch checks reject stale pointers after remap.
// Reason: Transferring read-only guard ownership across threads preserves invariants.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for VectorSliceGuard<'_> {}
// SAFETY: `VectorSliceGuard` is `Sync` because shared access is immutable.
// - Condition 1: Exposed data is `&[f32]` only; no mutable alias is produced.
// - Condition 2: Underlying map lifetime is tied to `_guard` and epoch validation.
// Reason: Concurrent reads of stable mapped memory are sound.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Sync for VectorSliceGuard<'_> {}

impl VectorSliceGuard<'_> {
    /// Returns the vector data as a slice.
    ///
    /// # Errors
    ///
    /// Returns `Error::EpochMismatch` if the underlying mmap has been remapped
    /// since this guard was created, meaning the pointer is stale.
    #[inline]
    pub fn as_slice(&self) -> crate::error::Result<&[f32]> {
        // SAFETY: ptr and len were validated during construction,
        // and the guard ensures the mmap remains valid
        // Verify epoch – if the mmap was remapped the pointer is invalid
        let current = self.epoch_ptr.load(std::sync::atomic::Ordering::Acquire);
        if current != self.epoch_at_creation {
            return Err(crate::error::Error::EpochMismatch(
                "Mmap was remapped; VectorSliceGuard is invalid".to_string(),
            ));
        }
        // SAFETY: `from_raw_parts` requires a valid pointer/len pair.
        // - Condition 1: `ptr` and `len` were validated when guard was created.
        // - Condition 2: Epoch equality above guarantees no remap invalidated `ptr`.
        // Reason: Zero-copy slice access avoids allocations while preserving safety invariants.
        Ok(unsafe { std::slice::from_raw_parts(self.ptr, self.len) })
    }
}

impl AsRef<[f32]> for VectorSliceGuard<'_> {
    #[inline]
    fn as_ref(&self) -> &[f32] {
        self.as_slice().expect("epoch mismatch in AsRef")
    }
}

impl std::ops::Deref for VectorSliceGuard<'_> {
    type Target = [f32];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice().expect("epoch mismatch in Deref")
    }
}
