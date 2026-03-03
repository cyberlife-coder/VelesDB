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
#![allow(clippy::doc_markdown)] // API names and parameter labels are kept verbatim in docs.

mod batch;
mod brute_force;
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
use std::sync::atomic::AtomicU64;

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
/// # Implementation Notes (v1.0+)
///
/// Since v1.0, `HnswInner` is `NativeHnswInner` — a fully owned, native
/// implementation with no mmap borrowing and no self-referential lifetimes.
/// `io_holder` is now `Option<Box<()>>` (always `None`) and is retained only
/// to preserve the field layout tested by `test_field_order_io_holder_after_inner`.
///
/// `ManuallyDrop` and the custom `Drop` are also kept for forward-compatibility:
/// if a future backend reintroduces borrowed data from disk, the invariant is
/// already enforced structurally without code changes.
///
/// # Drop Order Invariant
///
/// `inner` (HNSW graph) **must** be dropped before `io_holder`.
/// This is guaranteed by:
/// 1. `ManuallyDrop<HnswInner>` preventing automatic drop of `inner`.
/// 2. The explicit `Drop` impl calling `ManuallyDrop::drop` first.
/// 3. `io_holder` being declared **after** `inner` (enforced by the safety test).
pub struct HnswIndex {
    /// Vector dimension
    pub(crate) dimension: usize,
    /// Distance metric
    pub(crate) metric: DistanceMetric,
    /// Internal HNSW index.
    ///
    /// Wrapped in `ManuallyDrop` to control drop order. MUST be dropped
    /// BEFORE `io_holder` (see Drop Order Invariant in struct-level doc).
    /// Currently `NativeHnswInner` owns all its data, so no borrowing occurs;
    /// `ManuallyDrop` is retained for forward-compatibility.
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
    /// Optional soft latency target for two-stage reranking (microseconds).
    ///
    /// `0` disables latency-aware rerank adaptation.
    pub(crate) rerank_latency_target_us: AtomicU64,
    /// Exponential moving average of two-stage rerank latency (microseconds).
    pub(crate) rerank_latency_ema_us: AtomicU64,
    /// Reserved for future backends that may borrow from disk-mapped data.
    ///
    /// Always `None` with the native implementation. Declared AFTER `inner`
    /// so that if a borrowing backend is ever reintroduced, the drop-order
    /// invariant is already structurally enforced.
    #[allow(dead_code)] // Retained for field-layout invariant; dropped after inner
    pub(crate) io_holder: Option<Box<HnswIo>>,
}

impl Drop for HnswIndex {
    fn drop(&mut self) {
        // SAFETY: ManuallyDrop::drop requires exclusive ownership and a single call.
        // - Condition 1: `inner` is wrapped in ManuallyDrop to suppress automatic drop.
        // - Condition 2: Write lock guarantees no concurrent access during drop.
        // - Condition 3: This Drop impl is the only site that calls ManuallyDrop::drop.
        // Reason: Drop order invariant — `inner` must be destroyed before `io_holder`
        // to remain forward-compatible with backends that borrow from io_holder.
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
