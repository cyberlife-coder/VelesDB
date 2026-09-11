//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides a high-performance approximate nearest neighbor
//! search index based on the HNSW algorithm.
//!
//! # Quality Profiles
//!
//! The index supports different quality profiles for search:
//! - `Fast`: `ef_search=96`, 97.4% recall@10 in `recall_benchmark` (10K
//!   random 128-D points), lowest latency
//! - `Balanced`: `ef_search=160`, 99.8% recall@10 there, good tradeoff
//!   (default)
//! - `Accurate`: `ef_search=512`, 100% recall@10 there and 0.98 on SIFT1M's
//!   1M (`docs/BENCHMARKS.md`), high precision
//! - `Perfect`: an exhaustive scan that leaves the graph, returning the
//!   exact top-k under the index's own distance, ties aside, at O(n). This
//!   type does not cap it; a collection refuses it above
//!   `limits.max_perfect_mode_vectors`
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
mod rerank;
mod search;
mod trait_impl;
mod vacuum;

#[allow(unused_imports)]
// Re-export for downstream consumers; not directly used in this module
pub use vacuum::VacuumError;

use super::native_inner::NativeHnswInner as HnswInner;
use super::sharded_mappings::ShardedMappings;
use super::upsert;
use crate::distance::DistanceMetric;
use parking_lot::RwLock;
use std::mem::ManuallyDrop;
use std::sync::atomic::AtomicU64;

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
    /// Whether exact-distance features are enabled: the automatic two-stage
    /// re-rank (`search_with_quality`, `search_batch_parallel`), the exact
    /// scan those two run for `Perfect` and on an index of at most 100
    /// vectors, `search_brute_force`, `brute_force_search_parallel`, the GPU
    /// scans and vacuum. With it off, `search_with_rerank*` still re-rank
    /// (`search_with_rerank` then takes the caller's `rerank_k` candidates
    /// rather than the pool the index sizes), and `full_scan_with_bitmap`
    /// scans regardless.
    ///
    /// Vectors always live once, in the graph's `ContiguousVectors` (the
    /// former `ShardedVectors` sidecar was removed — PERF1). This flag is
    /// kept as a feature gate so fast-insert indices preserve their
    /// historical behavior (none of the features above).
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
    /// Directory this index may keep its f32 arena file in.
    ///
    /// Remembered rather than passed per call because the index rebuilds
    /// itself: `vacuum` constructs a whole replacement graph, and without
    /// this the replacement would come back heap-backed and silently undo
    /// the mapping on every compaction (#2112).
    ///
    /// `None` for an index with no collection directory, and honoured only
    /// by the quantized backends.
    pub(crate) arena_dir: Option<std::path::PathBuf>,
}

impl HnswIndex {
    /// Inserts a vector into the HNSW graph, then maps `id` to the slot the
    /// graph gave it (#2246).
    ///
    /// The graph owns the arena's allocation; the mapping only ever follows a
    /// slot already holding this vector, so no slot is predicted and none can
    /// be handed to two ids. Returns `false` when the graph refused the vector:
    /// no mapping was touched, so there is nothing to roll back.
    pub(crate) fn insert_and_assign(&self, id: u64, vector: &[f32]) -> bool {
        // Held until the id is mapped: `reorder_for_locality` and `vacuum`
        // renumber slots under the write lock, so the slot placed here is still
        // its vector's when the mapping names it.
        let inner = self.inner.read();
        let placed = inner
            .place(vector)
            .map(|placed| self.mappings.assign(id, placed));
        drop(inner);
        match placed {
            Ok(_) => true,
            Err(e) => {
                tracing::error!("HnswIndex::insert failed for id={id}: {e}");
                false
            }
        }
    }

    /// Removes a vector by ID (soft delete).
    ///
    /// Returns `true` if the ID existed and was removed from the mappings,
    /// `false` if the ID was absent. The HNSW graph node is not physically
    /// deleted — it becomes a tombstone, filtered out on search via the
    /// reverse mapping.
    ///
    /// For workloads with many deletions, consider periodic
    /// [`Self::vacuum`] to rebuild the graph and reclaim memory.
    ///
    /// This is the single inherent implementation shared by the
    /// [`VectorIndex::remove`](crate::index::VectorIndex::remove) trait impl
    /// and delegates to `upsert::soft_delete` (private; also used by
    /// `NativeHnswIndex::remove`).
    ///
    /// Holds the graph's read guard across both map writes: a renumber
    /// re-maps under the write guard and must not run between them.
    pub fn remove(&self, id: u64) -> bool {
        let graph = self.inner.read();
        upsert::soft_delete(&self.mappings, id, &graph)
    }

    /// Returns the number of vector slots in the graph's `ContiguousVectors`
    /// (live vectors + tombstones).
    ///
    /// This is the exclusive upper bound for internal indices: brute-force
    /// guards and rerank paths read vectors from this store, keyed by the
    /// mapping's internal index.
    pub(crate) fn graph_vector_count(&self) -> usize {
        self.inner
            .read()
            .with_contiguous_vectors(crate::perf_optimizations::ContiguousVectors::len)
    }

    /// Reorders graph nodes in BFS traversal order for improved cache locality.
    ///
    /// After reordering, vectors that are close in the graph are also close
    /// in memory, reducing cache misses during search traversal by 15-30%.
    ///
    /// Automatically skipped for small indices (< 1000 vectors) where the
    /// entire working set fits in L2 cache.
    ///
    /// # Why the write lock
    ///
    /// The pass renumbers every node, and `mappings` is keyed by that
    /// numbering, so the two are only consistent together. A read guard would
    /// let a search run between them and resolve external ids against the old
    /// numbering — confident, wrong answers rather than an error. `vacuum`
    /// takes the write lock to renumber for the same reason. `ANALYZE` is the
    /// caller, so the exclusion lasts one maintenance pass.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage reordering fails.
    pub fn reorder_for_locality(&self) -> crate::error::Result<()> {
        let guard = self.inner.write();
        let reordered = guard.reorder_for_locality()?;
        if let Some(old_to_new) = reordered {
            self.mappings.remap_indices(&old_to_new);
        }
        drop(guard);
        Ok(())
    }
}

impl Drop for HnswIndex {
    fn drop(&mut self) {
        // SAFETY: ManuallyDrop::drop requires exclusive ownership and a single call.
        // - Condition 1: `inner` is wrapped in ManuallyDrop to suppress automatic drop.
        // - Condition 2: Write lock guarantees no concurrent access during drop.
        // - Condition 3: This Drop impl is the only site that calls ManuallyDrop::drop.
        // SAFETY: Drop order invariant — `inner` must be destroyed before `io_holder`
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
#[path = "safety_tests.rs"]
mod safety_tests;
