//! Zero-copy vector reference abstraction.
//!
//! This module provides the `VectorRef` trait for zero-copy access to vectors,
//! eliminating heap allocations during search operations.
//!
//! # Performance
//!
//! Using `VectorRef` instead of `Vec<f32>` eliminates:
//! - **Heap allocations**: 0 allocations per read vs ~10k for 10k vector search
//! - **Memory copies**: Direct slice access from mmap
//! - **Allocator pressure**: No fragmentation from repeated alloc/dealloc
//!
//! # EPIC-B: TS-MEM-001, TS-MEM-002

use std::borrow::Cow;
use std::ops::Deref;

/// A reference to vector data that may be borrowed or owned.
///
/// This trait abstracts over different ways to access vector data:
/// - `&[f32]`: Direct slice reference (zero-copy from mmap)
/// - `Cow<[f32]>`: Copy-on-write for flexibility
/// - `Vec<f32>`: Owned data when needed
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::VectorRef;
///
/// fn compute_distance<V: VectorRef>(a: &V, b: &V) -> f32 {
///     let a_slice = a.as_slice();
///     let b_slice = b.as_slice();
///     // SIMD distance calculation on slices
///     crate::simd_native::cosine_similarity_native(a_slice, b_slice)
/// }
/// ```
pub trait VectorRef {
    /// Returns the vector data as a slice.
    fn as_slice(&self) -> &[f32];

    /// Returns the dimension of the vector.
    fn dimension(&self) -> usize {
        self.as_slice().len()
    }

    /// Returns true if the vector is empty.
    fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }
}

// ============================================================================
// Implementations for common types
// ============================================================================

impl VectorRef for [f32] {
    #[inline]
    fn as_slice(&self) -> &[f32] {
        self
    }
}

impl VectorRef for Vec<f32> {
    #[inline]
    fn as_slice(&self) -> &[f32] {
        self
    }
}

impl VectorRef for &[f32] {
    #[inline]
    fn as_slice(&self) -> &[f32] {
        self
    }
}

impl VectorRef for Cow<'_, [f32]> {
    #[inline]
    fn as_slice(&self) -> &[f32] {
        self
    }
}

/// A borrowed vector reference with explicit lifetime.
///
/// This is useful when you need to return a reference from a function
/// while keeping the source locked.
#[derive(Debug, Clone, Copy)]
pub struct BorrowedVector<'a> {
    data: &'a [f32],
}

impl<'a> BorrowedVector<'a> {
    /// Creates a new borrowed vector reference.
    #[inline]
    #[must_use]
    pub const fn new(data: &'a [f32]) -> Self {
        Self { data }
    }

    /// Returns the underlying slice.
    #[inline]
    #[must_use]
    pub const fn data(&self) -> &'a [f32] {
        self.data
    }
}

impl VectorRef for BorrowedVector<'_> {
    #[inline]
    fn as_slice(&self) -> &[f32] {
        self.data
    }
}

impl Deref for BorrowedVector<'_> {
    type Target = [f32];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl AsRef<[f32]> for BorrowedVector<'_> {
    #[inline]
    fn as_ref(&self) -> &[f32] {
        self.data
    }
}

/// Guard that holds a read lock and provides vector access.
///
/// This is used for zero-copy access from storage while holding the lock.
/// The guard ensures the underlying data remains valid.
pub struct VectorGuard<'a, G> {
    /// The lock guard (kept alive to hold the lock)
    _guard: G,
    /// Pointer to the vector data
    data: &'a [f32],
}

impl<'a, G> VectorGuard<'a, G> {
    /// Creates a new vector guard.
    ///
    /// # Safety
    ///
    /// The `data` pointer must remain valid as long as `guard` is held.
    /// This is enforced by the lifetime parameter.
    #[must_use]
    pub const fn new(guard: G, data: &'a [f32]) -> Self {
        Self {
            _guard: guard,
            data,
        }
    }
}

impl<G> VectorRef for VectorGuard<'_, G> {
    #[inline]
    fn as_slice(&self) -> &[f32] {
        self.data
    }
}

impl<G> Deref for VectorGuard<'_, G> {
    type Target = [f32];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<G> AsRef<[f32]> for VectorGuard<'_, G> {
    #[inline]
    fn as_ref(&self) -> &[f32] {
        self.data
    }
}

// ============================================================================
// Tests moved to vector_ref_tests.rs per project rules
