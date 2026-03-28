//! Object pools and compact data structures for HNSW search.
//!
//! Contains [`BitVecVisited`] (compact visited-node tracker), thread-local
//! pools for search heaps and bitsets, and helper type aliases shared by
//! the search subsystem.

use super::super::layer::NodeId;
use super::super::ordered_float::OrderedFloat;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

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
