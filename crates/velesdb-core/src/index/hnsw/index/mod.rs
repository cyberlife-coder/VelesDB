//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides a high-performance approximate nearest neighbor
//! search index based on the HNSW algorithm.
//!
//! # Quality Profiles
//!
//! The index supports different quality profiles for search:
//! - `Fast`: `ef_search=64`, ~92% recall, lowest latency
//! - `Balanced`: `ef_search=128`, ~99% recall, good tradeoff (default)
//! - `Accurate`: `ef_search=256`, ~100% recall, high precision
//! - `Perfect`: `ef_search=2048`, 100% recall, maximum accuracy
//!
//! # Recommended Parameters by Vector Dimension
//!
//! | Dimension   | M     | ef_construction | ef_search |
//! |-------------|-------|-----------------|-----------|
//! | d ≤ 256     | 12-16 | 100-200         | 64-128    |
//! | 256 < d ≤768| 16-24 | 200-400         | 128-256   |
//! | d > 768     | 24-32 | 300-600         | 256-512   |

mod batch;
mod constructors;
mod search;
mod trait_impl;
mod vacuum;

// Re-export VacuumError for public API
#[allow(unused_imports)]
pub use vacuum::VacuumError;

use super::native_inner::NativeHnswInner as HnswInner;
use super::sharded_mappings::ShardedMappings;
use super::sharded_vectors::ShardedVectors;
use crate::distance::DistanceMetric;
use parking_lot::RwLock;
use std::mem::ManuallyDrop;

// Native persistence - no HnswIo needed (v1.0+)
type HnswIo = ();

/// HNSW index for efficient approximate nearest neighbor search.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::index::HnswIndex;
/// use velesdb_core::DistanceMetric;
///
/// let index = HnswIndex::new(768, DistanceMetric::Cosine);
/// index.insert(1, &vec![0.1; 768]);
/// let results = index.search(&vec![0.1; 768], 10);
/// ```
/// HNSW index for efficient approximate nearest neighbor search.
///
/// # Safety Invariants (Self-Referential Pattern)
///
/// When loaded from disk via [`HnswIndex::load`], this struct uses a
/// self-referential pattern where `inner` (the HNSW graph) borrows from
/// `io_holder` (the memory-mapped file). This requires careful lifetime
/// management:
///
/// 1. **Field Order**: `io_holder` must be declared AFTER `inner` so Rust's
///    default drop order drops `inner` first (fields drop in declaration order).
///
/// 2. **`ManuallyDrop`**: `inner` is wrapped in `ManuallyDrop` so we can
///    explicitly control when it's dropped in our `Drop` impl.
///
/// 3. **Custom Drop**: Our `Drop` impl explicitly drops `inner` before
///    returning, ensuring `io_holder` (dropped automatically after) outlives it.
///
/// 4. **Lifetime Extension**: We use `'static` lifetime in `HnswInner` which is
///    technically a lie - the actual lifetime is tied to `io_holder`. This is
///    safe because we guarantee `io_holder` outlives `inner` via the above.
///
/// **Note**: The `ouroboros` crate cannot be used here because `hnsw_rs::Hnsw`
/// has an invariant lifetime parameter, which is incompatible with self-referential
/// struct crates that require covariant lifetimes.
///
/// # Feature Flags (v0.8.12+)
///
/// - `native-hnsw` (default): Uses native HNSW implementation (faster, no deps)
/// - `legacy-hnsw`: Uses `hnsw_rs` library for compatibility
///
/// # Why Not Unsafe Alternatives?
///
/// - `ouroboros`/`self_cell`: Require covariant lifetimes (Hnsw is invariant)
/// - `rental`: Deprecated and unmaintained
/// - `owning_ref`: Doesn't support this pattern
///
/// The current approach is a well-documented Rust pattern for handling libraries
/// that return borrowed data from owned resources.
pub struct HnswIndex {
    /// Vector dimension
    pub(crate) dimension: usize,
    /// Distance metric
    pub(crate) metric: DistanceMetric,
    /// Internal HNSW index (type-erased for flexibility).
    ///
    /// # Safety
    ///
    /// Wrapped in `ManuallyDrop` to control drop order. MUST be dropped
    /// BEFORE `io_holder` because it contains references into `io_holder`'s
    /// memory-mapped data (when loaded from disk).
    pub(crate) inner: RwLock<ManuallyDrop<HnswInner>>,
    /// ID mappings (external ID <-> internal index) - lock-free via `DashMap` (EPIC-A.1)
    pub(crate) mappings: ShardedMappings,
    /// Vector storage for SIMD re-ranking - sharded for parallel writes (EPIC-A.2)
    pub(crate) vectors: ShardedVectors,
    /// Whether to store vectors in `ShardedVectors` for re-ranking.
    ///
    /// When `false`, vectors are only stored in HNSW graph, providing:
    /// - ~2x faster insert throughput
    /// - ~50% less memory usage
    /// - No SIMD re-ranking or brute-force search support
    ///
    /// Default: `true` (full functionality)
    pub(crate) enable_vector_storage: bool,
    /// Holds the `HnswIo` for loaded indices.
    ///
    /// # Safety
    ///
    /// This field MUST be declared AFTER `inner` and MUST outlive `inner`.
    /// The `Hnsw` in `inner` borrows from the memory-mapped data owned by `HnswIo`.
    /// Our `Drop` impl ensures `inner` is dropped first.
    ///
    /// - `Some(Box<HnswIo>)`: Index was loaded from disk, `inner` borrows from this
    /// - `None`: Index was created in memory, no borrowing relationship
    #[allow(dead_code)] // Read implicitly via lifetime - dropped after inner
    pub(crate) io_holder: Option<Box<HnswIo>>,
}

impl Drop for HnswIndex {
    fn drop(&mut self) {
        // SAFETY: We must drop inner BEFORE io_holder because inner (HnswInner)
        // borrows from io_holder. ManuallyDrop lets us control this order.
        // - Condition 1: inner is wrapped in ManuallyDrop to prevent automatic drop.
        // - Condition 2: We acquire a write lock before dropping to ensure exclusive access.
        // - Condition 3: ManuallyDrop::drop is only called once in this Drop impl.
        // - Condition 4: For loaded indices, io_holder outlives inner; for new indices, io_holder is None.
        // Reason: Self-referential struct pattern requires manual drop order control to prevent use-after-free.
        unsafe {
            ManuallyDrop::drop(&mut *self.inner.write());
        }
        // io_holder will be dropped automatically after this function returns
    }
}

// ============================================================================
// Safety tests - must stay in this file (require private field access)
// ============================================================================
#[cfg(test)]
mod safety_tests {
    use super::*;

    /// Compile-time assertion that `io_holder` field is declared AFTER `inner`.
    #[test]
    fn test_field_order_io_holder_after_inner() {
        use std::mem::offset_of;

        let inner_offset = offset_of!(HnswIndex, inner);
        let io_holder_offset = offset_of!(HnswIndex, io_holder);

        assert!(
            inner_offset < io_holder_offset,
            "CRITICAL: io_holder must be declared AFTER inner for correct drop order"
        );
    }
}
