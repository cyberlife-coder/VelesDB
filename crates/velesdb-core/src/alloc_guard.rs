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

use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Default ceiling for a single raw allocation, in bytes: **1 TiB**.
///
/// # Rationale (#899 + follow-up)
///
/// This is purely a *backstop against pathological / attacker-controlled /
/// overflow-class sizes*, **not** a workload limit. It must NEVER reject a
/// legitimate large index.
///
/// `ContiguousVectors` is a **single monolithic buffer holding ALL vectors of an
/// HNSW graph** — it is not sharded. The previous 16 GiB ceiling therefore
/// falsely rejected legitimate ingests: a collection only needs ~5.6M vectors at
/// 768D to exceed 16 GiB in one buffer, and the geometric doubling in
/// `ensure_capacity`/`resize` tripped even earlier (~2.8M @768D, when the next
/// doubling crosses 16 GiB). Worse, an index built+persisted under the old code
/// with > 16 GiB of vectors became **un-loadable** after upgrade.
///
/// 1 TiB is chosen because:
/// - It is far above any single in-RAM vector buffer a real deployment builds
///   (~358M vectors at 768D, ~715M at 384D), so legitimate workloads never trip
///   it; the OS allocator / OOM killer rejects genuinely-impossible sizes first.
/// - It still sits *vastly* below overflow-class requests: a wrapped `usize`
///   lands near `usize::MAX` (~16 EiB on 64-bit), four-plus orders of magnitude
///   above 1 TiB, so wrapped/absurd sizes are still cut off before they reach the
///   system allocator.
/// - The real overflow guard is the `checked_mul`/`checked_add` arithmetic in
///   [`ContiguousVectors::byte_size`](crate::perf_optimizations::ContiguousVectors)
///   and the insert/resize paths; this ceiling is a coarse secondary net.
///
/// Configurable at runtime via [`set_alloc_byte_limit`] if an operator
/// legitimately needs an even larger single allocation, or to harden it down.
pub const DEFAULT_ALLOC_BYTE_LIMIT: usize = 1024 * 1024 * 1024 * 1024;

/// Process-wide per-allocation byte ceiling, initialized to
/// [`DEFAULT_ALLOC_BYTE_LIMIT`]. See [`set_alloc_byte_limit`].
static ALLOC_BYTE_LIMIT: AtomicUsize = AtomicUsize::new(DEFAULT_ALLOC_BYTE_LIMIT);

/// Overrides the per-allocation byte ceiling enforced by [`AllocGuard`].
///
/// Use to raise the limit for genuinely huge single-buffer workloads, or lower
/// it to harden against pathological sizes. Affects all subsequent allocations
/// through [`AllocGuard::new`] / [`AllocGuard::new_zeroed`] and
/// [`check_alloc_bound`]. Passing `0` is treated as "no override" and restores
/// the default ceiling.
pub fn set_alloc_byte_limit(limit_bytes: usize) {
    let effective = if limit_bytes == 0 {
        DEFAULT_ALLOC_BYTE_LIMIT
    } else {
        limit_bytes
    };
    ALLOC_BYTE_LIMIT.store(effective, Ordering::Relaxed);
}

/// Returns the current per-allocation byte ceiling.
#[must_use]
pub fn alloc_byte_limit() -> usize {
    ALLOC_BYTE_LIMIT.load(Ordering::Relaxed)
}

/// Runs `f` with the per-allocation ceiling raised to **at least** `min_bytes`,
/// restoring the previous ceiling afterward (even on panic).
///
/// Used by the persisted-index **load** path: the on-disk vector payload has
/// already been validated to fit within the actual file length (see
/// `validate_vectors_file_len`), so it is a *real, legitimately-built* size — it
/// must reload regardless of the process-wide backstop. Bounding the temporary
/// raise by the file-derived `min_bytes` (rather than removing the ceiling) keeps
/// the backstop meaningful: a corrupt oversized header is still rejected earlier
/// by the file-length check, and unrelated allocations during `f` are still
/// bounded by `max(previous_limit, min_bytes)`.
///
/// If the current ceiling already covers `min_bytes`, this is a transparent
/// pass-through with no mutation.
pub fn with_min_alloc_byte_limit<T>(min_bytes: usize, f: impl FnOnce() -> T) -> T {
    let previous = alloc_byte_limit();
    if min_bytes <= previous {
        return f();
    }
    // RAII restore so a panic inside `f` cannot leave the ceiling raised.
    let _restore = LimitRestore(previous);
    ALLOC_BYTE_LIMIT.store(min_bytes, Ordering::Relaxed);
    f()
}

/// RAII helper that restores [`ALLOC_BYTE_LIMIT`] to a saved value on drop,
/// including during unwinding. Used by [`with_min_alloc_byte_limit`].
struct LimitRestore(usize);

impl Drop for LimitRestore {
    fn drop(&mut self) {
        ALLOC_BYTE_LIMIT.store(self.0, Ordering::Relaxed);
    }
}

/// Validates that a requested allocation of `bytes` is within the configured
/// ceiling, *without* allocating.
///
/// Lets callers reject pathological sizes before building a [`Layout`] or
/// reserving a `Vec`. Thread this in front of resize / gather / reorder paths.
///
/// # Errors
///
/// Returns [`Error::AllocationFailed`](crate::error::Error::AllocationFailed) if
/// `bytes` exceeds [`alloc_byte_limit`].
pub fn check_alloc_bound(bytes: usize) -> crate::error::Result<()> {
    let limit = alloc_byte_limit();
    if bytes > limit {
        return Err(crate::error::Error::AllocationFailed(format!(
            "requested allocation of {bytes} bytes exceeds ceiling of {limit} bytes \
             (raise via set_alloc_byte_limit if intentional)"
        )));
    }
    Ok(())
}

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
    /// - `None` if allocation failed (OOM), layout size is zero, or the request
    ///   exceeds the configured [`alloc_byte_limit`] (#899 backstop)
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
        if layout.size() == 0 || layout.size() > alloc_byte_limit() {
            return None;
        }

        // SAFETY: `alloc` requires a valid non-zero layout.
        // - Condition 1: `layout.size() > 0` is checked above.
        // - Condition 2: `Layout` comes from std APIs and is therefore well-formed.
        // SAFETY: Raw allocation is required to build a panic-safe RAII guard.
        let ptr = unsafe { alloc(layout) };

        NonNull::new(ptr).map(|ptr| Self {
            ptr,
            layout,
            owns_memory: true,
        })
    }

    /// Allocates zero-initialized memory with the given layout.
    ///
    /// Same as [`new`](Self::new) but guarantees all bytes are zero.
    /// Use for buffers where sparse writes (e.g., `insert_at`) may leave gaps.
    ///
    /// Returns `None` when the layout size is zero or exceeds the configured
    /// [`alloc_byte_limit`] (#899 backstop).
    #[must_use]
    pub fn new_zeroed(layout: Layout) -> Option<Self> {
        if layout.size() == 0 || layout.size() > alloc_byte_limit() {
            return None;
        }

        // SAFETY: `alloc_zeroed` requires a valid non-zero layout.
        // - Condition 1: `layout.size() > 0` is checked above.
        // - Condition 2: `Layout` comes from std APIs and is therefore well-formed.
        // SAFETY: Zero-initialized allocation prevents UB from reading unwritten slots.
        let ptr = unsafe { alloc_zeroed(layout) };

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
            // SAFETY: `dealloc` requires the original pointer/layout pair.
            // - Condition 1: `self.ptr` was produced by `alloc(self.layout)` in `new`.
            // - Condition 2: `owns_memory` guarantees this path runs at most once.
            // SAFETY: Manual deallocation is needed for raw-memory RAII.
            unsafe {
                dealloc(self.ptr.as_ptr(), self.layout);
            }
        }
    }
}

// SAFETY: `AllocGuard` is `Send` because it owns an allocation handle only.
// - Condition 1: No aliasing references are stored, only pointer + layout metadata.
// - Condition 2: Mutation requires `&mut self`, preventing cross-thread races on the type.
// SAFETY: Heap allocations are not thread-affine; ownership transfer across threads is sound.
unsafe impl Send for AllocGuard {}

// AllocGuard is NOT Sync - concurrent access to raw memory is unsafe
// (intentionally not implementing Sync)
