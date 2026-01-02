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
///     crate::simd::cosine_similarity_fast(a_slice, b_slice)
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
// TDD TESTS
// ============================================================================

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // VectorRef trait tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_vector_ref_slice() {
        // Arrange
        let data: &[f32] = &[1.0, 2.0, 3.0];

        // Act & Assert
        assert_eq!(data.as_slice(), &[1.0, 2.0, 3.0]);
        assert_eq!(data.dimension(), 3);
        assert!(!data.is_empty());
    }

    #[test]
    fn test_vector_ref_vec() {
        // Arrange
        let data: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];

        // Act & Assert
        assert_eq!(data.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(data.dimension(), 4);
    }

    #[test]
    fn test_vector_ref_cow_borrowed() {
        // Arrange
        let original = vec![1.0, 2.0];
        let cow: Cow<[f32]> = Cow::Borrowed(&original);

        // Act & Assert
        assert_eq!(cow.as_slice(), &[1.0, 2.0]);
        assert_eq!(cow.dimension(), 2);
    }

    #[test]
    fn test_vector_ref_cow_owned() {
        // Arrange
        let cow: Cow<[f32]> = Cow::Owned(vec![1.0, 2.0, 3.0]);

        // Act & Assert
        assert_eq!(cow.as_slice(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_vector_ref_empty() {
        // Arrange
        let data: &[f32] = &[];

        // Act & Assert
        assert!(data.is_empty());
        assert_eq!(data.dimension(), 0);
    }

    // -------------------------------------------------------------------------
    // BorrowedVector tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_borrowed_vector_new() {
        // Arrange
        let data = [1.0f32, 2.0, 3.0];

        // Act
        let borrowed = BorrowedVector::new(&data);

        // Assert
        assert_eq!(borrowed.data(), &[1.0, 2.0, 3.0]);
        assert_eq!(borrowed.dimension(), 3);
    }

    #[test]
    fn test_borrowed_vector_deref() {
        // Arrange
        let data = [1.0f32, 2.0, 3.0];
        let borrowed = BorrowedVector::new(&data);

        // Act - use Deref trait
        let sum: f32 = borrowed.iter().sum();

        // Assert
        assert_eq!(sum, 6.0);
    }

    #[test]
    fn test_borrowed_vector_as_ref() {
        // Arrange
        let data = [1.0f32, 2.0];
        let borrowed = BorrowedVector::new(&data);

        // Act
        let slice: &[f32] = borrowed.as_ref();

        // Assert
        assert_eq!(slice, &[1.0, 2.0]);
    }

    // -------------------------------------------------------------------------
    // VectorGuard tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_vector_guard_basic() {
        // Arrange - simulate a lock guard with a simple value
        let data = [1.0f32, 2.0, 3.0, 4.0];
        let guard = (); // Dummy guard

        // Act
        let vector_guard = VectorGuard::new(guard, &data);

        // Assert
        assert_eq!(vector_guard.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(vector_guard.dimension(), 4);
    }

    #[test]
    fn test_vector_guard_deref() {
        // Arrange
        let data = [1.0f32, 2.0, 3.0];
        let guard = VectorGuard::new((), &data);

        // Act - use Deref to iterate
        let max = guard.iter().copied().fold(f32::NEG_INFINITY, f32::max);

        // Assert
        assert_eq!(max, 3.0);
    }

    #[test]
    fn test_vector_guard_with_real_lock() {
        use parking_lot::RwLock;

        // Arrange - use a real RwLock with static data
        static DATA: [f32; 3] = [1.0, 2.0, 3.0];
        let lock = RwLock::new(());

        // Act - create guard that holds the lock
        let read_guard = lock.read();
        let vector_guard = VectorGuard::new(read_guard, &DATA);

        // Assert - can access data through guard
        assert_eq!(vector_guard.as_slice(), &[1.0, 2.0, 3.0]);
        // Lock is held until vector_guard is dropped
    }

    // -------------------------------------------------------------------------
    // Generic function tests
    // -------------------------------------------------------------------------

    fn generic_sum<V: VectorRef>(v: &V) -> f32 {
        v.as_slice().iter().sum()
    }

    #[test]
    fn test_generic_function_with_slice() {
        let data: &[f32] = &[1.0, 2.0, 3.0];
        assert_eq!(generic_sum(&data), 6.0);
    }

    #[test]
    fn test_generic_function_with_vec() {
        let data = vec![1.0f32, 2.0, 3.0, 4.0];
        assert_eq!(generic_sum(&data), 10.0);
    }

    #[test]
    fn test_generic_function_with_borrowed() {
        let data = [1.0f32, 2.0];
        let borrowed = BorrowedVector::new(&data);
        assert_eq!(generic_sum(&borrowed), 3.0);
    }

    #[test]
    fn test_generic_function_with_cow() {
        let cow: Cow<[f32]> = Cow::Owned(vec![1.0, 2.0, 3.0]);
        assert_eq!(generic_sum(&cow), 6.0);
    }
}
