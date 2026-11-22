//! Object pools and compact data structures for HNSW search.
//!
//! Contains [`BitVecVisited`] (compact visited-node tracker), thread-local
//! pools for search heaps, bitsets and prepared queries, and helper type aliases shared by
//! the search subsystem.

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::super::ordered_float::OrderedFloat;
use super::NativeHnsw;
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

// Thread-local reusable buffer for cosine query normalization.
//
// Avoids allocating a new `Vec<f32>` on every cosine search call.
// Pre-sized for 1536-dim (common embedding dimension). After the first
// search, subsequent searches reuse the same allocation (zero-alloc hot path).
thread_local! {
    pub(super) static QUERY_BUF: RefCell<Vec<f32>> = RefCell::new(Vec::with_capacity(1536));
}

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Runs `f` on a prepared (cosine-normalized) view of `query`, recycling
    /// the thread-local scratch buffer afterwards.
    ///
    /// For non-cosine (or non-pre-normalized) engines this is a zero-cost
    /// pass-through of the raw slice. For the pre-normalized cosine engine
    /// the query is normalized into the `QUERY_BUF` thread-local scratch,
    /// which is returned to the pool after `f` completes — so a batch loop
    /// calling this per element keeps at most **one** owned buffer alive per
    /// thread, instead of one per element (PERF2).
    ///
    /// The normalized bits are identical to those produced by
    /// [`Self::prepare_query`] and by the in-place storage normalization in
    /// the batch insert path: `normalize_inplace_native` dispatches on the
    /// global SIMD level and slice length only (unaligned loads), so the
    /// same input bits always yield the same output bits.
    #[inline]
    pub(in crate::index::hnsw) fn with_prepared_query<R>(
        &self,
        query: &[f32],
        f: impl FnOnce(&[f32]) -> R,
    ) -> R {
        let prepared = self.prepare_query(query);
        let result = f(&prepared);
        Self::recycle_cow(prepared);
        result
    }

    /// Returns a query buffer to the thread-local pool for reuse.
    ///
    /// Called after the prepared query is no longer needed. This avoids
    /// deallocation so the next `prepare_query` call is zero-alloc.
    #[inline]
    fn return_query_buf(buf: Vec<f32>) {
        QUERY_BUF.with(|cell| {
            let mut borrow = cell.borrow_mut();
            if borrow.is_empty() {
                *borrow = buf;
            }
            // If the thread-local already has a buffer (e.g., concurrent
            // reentrant use), silently drop the extra one.
        });
    }

    /// Returns a `Cow`'s owned buffer to the thread-local pool for reuse.
    ///
    /// If the `Cow` is `Borrowed`, this is a no-op. If `Owned`, the buffer
    /// is returned to `QUERY_BUF` so the next `prepare_query` call avoids
    /// allocation.
    #[inline]
    pub(super) fn recycle_cow(cow: Cow<'_, [f32]>) {
        if let Cow::Owned(buf) = cow {
            Self::return_query_buf(buf);
        }
    }
}

// =============================================================================
// BitVecVisited — compact visited-node tracker (Issue #420, Component 2)
// =============================================================================

/// Compact visited-node tracker using one bit per node ID.
///
/// For 10K nodes this uses 1.25 KB (fits in L1 cache), compared to
/// ~80 KB for `FxHashSet<usize>`. The bitset is stored as `Vec<u64>`
/// for efficient 64-bit word operations.
///
/// # Usage
///
/// ```text
/// let mut visited = BitVecVisited::with_capacity(10_000);
/// visited.insert(42);
/// assert!(visited.contains(42));
/// visited.clear();     // O(n/64) memset, preserves allocation
/// ```
#[derive(Default)]
pub(crate) struct BitVecVisited {
    /// Each bit at position `i` indicates whether node `i` has been visited.
    pub(in crate::index::hnsw::native::graph) words: Vec<u64>,
}

impl BitVecVisited {
    /// Creates a new `BitVecVisited` with enough capacity for node IDs in `[0, capacity)`.
    ///
    /// All bits are initially unset (not visited).
    #[inline]
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let word_count = capacity.div_ceil(64);
        Self {
            words: vec![0_u64; word_count],
        }
    }

    /// Returns `true` if the given node ID has been marked as visited.
    ///
    /// Returns `false` for IDs beyond the current capacity (no panic).
    #[cfg(test)]
    #[inline]
    pub(crate) fn contains(&self, id: usize) -> bool {
        let word_idx = id / 64;
        let bit_idx = id % 64;
        self.words
            .get(word_idx)
            .is_some_and(|word| word & (1_u64 << bit_idx) != 0)
    }

    /// Marks a node ID as visited.
    ///
    /// Returns `true` if the node was **not** previously visited (newly inserted),
    /// matching the `HashSet::insert` contract for drop-in replacement.
    ///
    /// Grows the internal storage if `id` exceeds the current capacity.
    #[inline]
    pub(crate) fn insert(&mut self, id: usize) -> bool {
        self.ensure_capacity(id);
        let word_idx = id / 64;
        let bit_idx = id % 64;
        let mask = 1_u64 << bit_idx;
        let was_unset = self.words[word_idx] & mask == 0;
        self.words[word_idx] |= mask;
        was_unset
    }

    /// Resets all bits to zero without deallocating.
    ///
    /// Uses `fill(0)` which compiles to a single `memset` — O(n/64)
    /// and far cheaper than dropping and reallocating.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.words.fill(0);
    }

    /// Grows the internal storage so that `id` fits, if it does not already.
    #[inline]
    pub(in crate::index::hnsw::native::graph) fn ensure_capacity(&mut self, id: usize) {
        let required = id / 64 + 1;
        if required > self.words.len() {
            self.words.resize(required, 0);
        }
    }
}

/// Returns whether prefetch hints should be used for vectors of the given dimension.
///
/// Threshold: vector must span at least 2 cache lines (128 bytes = 32 f32 elements).
/// Below this, prefetch overhead exceeds the benefit.
#[inline]
pub(super) fn should_prefetch(dimension: usize) -> bool {
    let vector_bytes = dimension * std::mem::size_of::<f32>();
    vector_bytes >= 2 * crate::simd_native::L2_CACHE_LINE_BYTES
}

/// Maximum number of pooled instances retained per thread.
///
/// Applies to visited bitsets, candidate heaps, and result heaps.
pub(super) const POOL_MAX: usize = 4;

/// Type alias for the candidate min-heap (closest candidate first).
pub(super) type CandidateHeap = BinaryHeap<Reverse<(OrderedFloat, NodeId)>>;

/// Type alias for the result max-heap (furthest result first for eviction).
pub(super) type ResultHeap = BinaryHeap<(OrderedFloat, NodeId)>;

// Thread-local pools of reusable search data structures to avoid repeated
// allocations during batch HNSW searches. Each thread keeps up to `POOL_MAX`
// instances of each type.
thread_local! {
    pub(super) static VISITED_POOL: RefCell<Vec<BitVecVisited>> = const { RefCell::new(Vec::new()) };
    pub(super) static CANDIDATE_HEAP_POOL: RefCell<Vec<CandidateHeap>> = const { RefCell::new(Vec::new()) };
    pub(super) static RESULT_HEAP_POOL: RefCell<Vec<ResultHeap>> = const { RefCell::new(Vec::new()) };
}

/// Borrows a visited bitset from the thread-local pool, or creates a new one.
///
/// `capacity_hint` is the current node count — the returned bitset is
/// guaranteed to hold at least that many bits. Pooled bitsets are grown
/// to `capacity_hint` if the index has expanded since they were returned.
#[inline]
pub(super) fn acquire_visited_set(capacity_hint: usize) -> BitVecVisited {
    VISITED_POOL.with(|pool| {
        let mut set = pool
            .borrow_mut()
            .pop()
            .unwrap_or_else(|| BitVecVisited::with_capacity(capacity_hint));
        // Ensure pooled bitsets are large enough for the current index size.
        if capacity_hint > 0 {
            set.ensure_capacity(capacity_hint.saturating_sub(1));
        }
        set
    })
}

/// Returns a visited bitset to the thread-local pool after clearing it.
///
/// Keeps at most [`POOL_MAX`] bitsets per thread to bound memory usage.
#[inline]
pub(super) fn release_visited_set(mut set: BitVecVisited) {
    set.clear();
    VISITED_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < POOL_MAX {
            pool.push(set);
        }
    });
}

/// Borrows a candidate heap from the thread-local pool, or creates a new one.
///
/// The returned heap is guaranteed to be empty (cleared before pooling).
#[inline]
pub(super) fn acquire_candidate_heap() -> CandidateHeap {
    CANDIDATE_HEAP_POOL.with(|pool| pool.borrow_mut().pop().unwrap_or_default())
}

/// Returns a candidate heap to the thread-local pool after clearing it.
///
/// Keeps at most [`POOL_MAX`] heaps per thread to bound memory usage.
#[inline]
pub(super) fn release_candidate_heap(mut heap: CandidateHeap) {
    heap.clear();
    CANDIDATE_HEAP_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < POOL_MAX {
            pool.push(heap);
        }
    });
}

/// Borrows a result heap from the thread-local pool, or creates a new one.
///
/// The returned heap is guaranteed to be empty (cleared before pooling).
#[inline]
pub(super) fn acquire_result_heap() -> ResultHeap {
    RESULT_HEAP_POOL.with(|pool| pool.borrow_mut().pop().unwrap_or_default())
}

/// Returns a result heap to the thread-local pool after clearing it.
///
/// Keeps at most [`POOL_MAX`] heaps per thread to bound memory usage.
#[inline]
pub(super) fn release_result_heap(mut heap: ResultHeap) {
    heap.clear();
    RESULT_HEAP_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < POOL_MAX {
            pool.push(heap);
        }
    });
}
