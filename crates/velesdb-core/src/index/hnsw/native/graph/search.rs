//! HNSW search operations — entry points and layer-level helpers.

use super::super::distance::{batch_distance_with_prefetch, DistanceEngine};
use super::super::layer::{Layer, NodeId};
use super::super::ordered_float::OrderedFloat;
use super::search_pools::should_prefetch;
use super::search_state::{gather_unvisited_neighbors, process_batch_results, SearchState};
use super::{NativeHnsw, NO_ENTRY_POINT};
use crate::perf_optimizations::ContiguousVectors;
use smallvec::SmallVec;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::sync::atomic::{AtomicU64, Ordering};

// Thread-local reusable buffer for cosine query normalization.
//
// Avoids allocating a new `Vec<f32>` on every cosine search call.
// Pre-sized for 1536-dim (common embedding dimension). After the first
// search, subsequent searches reuse the same allocation (zero-alloc hot path).
thread_local! {
    static QUERY_BUF: RefCell<Vec<f32>> = RefCell::new(Vec::with_capacity(1536));
}

/// Global counter for seeding per-thread probe RNGs (issue #967).
///
/// Each thread increments this exactly once at first use and never again.
/// `Relaxed` ordering suffices: we need distinct initial seeds, not ordering
/// guarantees between threads.
static PROBE_RNG_SEED_COUNTER: AtomicU64 = AtomicU64::new(0x5DEE_CE66_D1A4_B5B5);

thread_local! {
    /// Per-thread XORshift64 state for multi-probe entry-point selection.
    ///
    /// Seeded lazily from `PROBE_RNG_SEED_COUNTER` on first use, then
    /// advanced entirely in thread-local storage.  The steady-state search
    /// path therefore touches zero shared atomics for probe randomisation.
    static PROBE_RNG: Cell<u64> = const { Cell::new(0) };
}

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Searches for k nearest neighbors.
    ///
    /// # Distance semantics
    ///
    /// Returned distances are **raw engine distances** from `D::distance()`.
    /// When `D = CachedSimdDistance`, Euclidean values are squared L2 (no
    /// sqrt). Callers that expose results to users must apply
    /// `NativeHnsw::transform_score` (private) to convert to the
    /// user-visible metric.
    #[inline]
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        let prepared_query = self.prepare_query(query);
        let results = self.search_prepared(&prepared_query, k, ef_search);
        Self::recycle_cow(prepared_query);
        results
    }

    /// Executes the search on an already-prepared (normalized) query vector.
    ///
    /// Factored out of [`search`] so the `Cow` borrow ends before
    /// [`recycle_cow`] reclaims the buffer.
    #[inline]
    fn search_prepared(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        let ep = self.entry_point.load(Ordering::Acquire);
        if ep == NO_ENTRY_POINT {
            return Vec::new();
        }

        let max_layer = self.max_layer.load(Ordering::Relaxed);

        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        let count = self.count.load(Ordering::Relaxed);
        let probes = self.adaptive_num_probes(count, ef_search, k);

        if probes > 1 {
            self.search_multi_entry_prepared(query, k, ef_search, probes)
        } else {
            self.search_layer(
                query,
                &[current_ep],
                ef_search,
                0,
                self.stagnation_limit,
                Some(k),
            )
        }
    }

    /// Adaptive number of entry-point probes for high-recall searches.
    #[inline]
    #[allow(clippy::unused_self)] // Reason: method receiver accesses graph config in future adaptive tuning
    fn adaptive_num_probes(&self, count: usize, ef_search: usize, k: usize) -> usize {
        if count <= 10_000 || ef_search <= (k * 4).max(64) {
            return 1;
        }

        if ef_search >= 1024 {
            4
        } else if ef_search >= 512 {
            3
        } else {
            2
        }
    }

    /// Multi-entry point search for improved recall on hard queries.
    ///
    /// Normalizes the query for cosine metric before searching. If the query
    /// is already prepared (e.g., from [`Self::search`]), use the private
    /// `search_multi_entry_prepared` companion to avoid double normalization.
    #[must_use]
    pub fn search_multi_entry(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        num_probes: usize,
    ) -> Vec<(NodeId, f32)> {
        let prepared_query = self.prepare_query(query);
        let result = self.search_multi_entry_prepared(&prepared_query, k, ef_search, num_probes);
        Self::recycle_cow(prepared_query);
        result
    }

    /// Multi-entry point search on an already-prepared query vector.
    ///
    /// Skips the `prepare_query` step — the caller is responsible for
    /// normalization (cosine). Called internally by [`Self::search`] which
    /// prepares the query once at the top level.
    #[must_use]
    fn search_multi_entry_prepared(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        num_probes: usize,
    ) -> Vec<(NodeId, f32)> {
        let ep = self.entry_point.load(Ordering::Acquire);
        if ep == NO_ENTRY_POINT {
            return Vec::new();
        }

        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return Vec::new();
        }

        let max_layer = self.max_layer.load(Ordering::Relaxed);

        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        let entry_points = Self::gather_multi_entry_points(current_ep, count, num_probes);

        self.search_layer(
            query,
            &entry_points,
            ef_search,
            0,
            self.stagnation_limit,
            Some(k),
        )
    }

    /// Gathers multiple entry points by adding random probes alongside the
    /// greedy-descent entry point.
    ///
    /// Probe IDs are drawn from the **thread-local** XORshift64 RNG (issue #967).
    /// This eliminates the shared `rng_state.fetch_update` CAS that previously
    /// ran once per search on every thread simultaneously, causing cache-line
    /// bouncing proportional to the number of concurrent searchers.
    #[inline]
    fn gather_multi_entry_points(
        primary_ep: NodeId,
        count: usize,
        num_probes: usize,
    ) -> Vec<NodeId> {
        let mut entry_points = vec![primary_ep];
        if num_probes > 1 && count > 10 {
            for _ in 1..num_probes.min(4) {
                let random_id = (Self::next_probe_rng() as usize) % count;
                if !entry_points.contains(&random_id) {
                    entry_points.push(random_id);
                }
            }
        }
        entry_points
    }

    /// Advances the thread-local probe RNG and returns the next value.
    ///
    /// **First call per thread**: one `Relaxed` `fetch_add` on the global
    /// `PROBE_RNG_SEED_COUNTER` to obtain a unique starting seed.  This cost
    /// is amortised over millions of subsequent search calls.
    ///
    /// **All later calls**: pure thread-local XORshift64, touching no shared
    /// memory.  Eight concurrent threads therefore generate independent random
    /// sequences with zero inter-thread synchronisation.
    #[inline]
    fn next_probe_rng() -> u64 {
        PROBE_RNG.with(|cell| {
            let mut s = cell.get();
            if s == 0 {
                // One-time per-thread seed: stride the global counter by a
                // large odd constant so threads that initialise back-to-back
                // start at well-separated points in the XORshift cycle.
                s = PROBE_RNG_SEED_COUNTER.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
                if s == 0 {
                    s = 1; // XORshift64 must not start at 0
                }
            }
            let next = super::xorshift64(s);
            cell.set(next);
            next
        })
    }

    /// Returns a `Cow`'s owned buffer to the thread-local pool for reuse.
    ///
    /// If the `Cow` is `Borrowed`, this is a no-op. If `Owned`, the buffer
    /// is returned to `QUERY_BUF` so the next `prepare_query` call avoids
    /// allocation.
    #[inline]
    fn recycle_cow(cow: Cow<'_, [f32]>) {
        if let Cow::Owned(buf) = cow {
            Self::return_query_buf(buf);
        }
    }

    // =========================================================================
    // Layer-level search helpers
    // =========================================================================

    /// F-04 optimization: acquires both vectors and layers read locks once
    /// before the greedy descent loop, avoiding repeated lock cycles per hop.
    ///
    /// Includes software prefetch hints for upcoming neighbor vectors to
    /// reduce memory latency in upper HNSW layers (mirrors `search_layer`).
    #[inline]
    pub(in crate::index::hnsw::native::graph) fn search_layer_single(
        &self,
        query: &[f32],
        entry: NodeId,
        layer: usize,
    ) -> NodeId {
        self.with_vectors_and_layers_read(|vectors, layers| {
            let dimension = vectors.dimension();
            let prefetch_dist = crate::simd_native::calculate_prefetch_distance(dimension);
            let mut best = entry;
            debug_assert!(
                entry < vectors.len(),
                "entry {entry} out of bounds (len {})",
                vectors.len()
            );
            // SAFETY: `get_unchecked` dereferences `entry` without bounds checks.
            // - Condition 1: `entry < vectors.len()` verified by `debug_assert!` above.
            //   Persisted entry points / neighbor IDs are validated `< count`
            //   once at load time (`graph_io::validate_graph_header`,
            //   `read_node_neighbors`), so this holds in release too.
            // SAFETY: Skipping the bounds check avoids a branch in the HNSW hot path.
            let entry_vec = unsafe { vectors.get_unchecked(entry) };
            let mut best_dist = self.distance.distance(query, entry_vec);

            loop {
                let improved = layers[layer]
                    .with_neighbors(best, |neighbors| {
                        self.greedy_scan_with_prefetch(
                            query,
                            neighbors,
                            vectors,
                            dimension,
                            prefetch_dist,
                            &mut best,
                            &mut best_dist,
                        )
                    })
                    .unwrap_or(false);

                if !improved {
                    break;
                }
            }

            best
        })
    }

    /// Prefetch neighbor vectors into CPU cache ahead of access.
    #[inline]
    fn prefetch_neighbors(
        neighbors: &[NodeId],
        vectors: &crate::perf_optimizations::ContiguousVectors,
        start: usize,
        count: usize,
    ) {
        for &neighbor_id in neighbors.iter().skip(start).take(count) {
            if neighbor_id < vectors.len() {
                vectors.prefetch(neighbor_id);
            }
        }
    }

    /// Scans a neighbor list with software prefetch, updating best node/dist.
    ///
    /// Returns `true` if a closer neighbor was found during the scan.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn greedy_scan_with_prefetch(
        &self,
        query: &[f32],
        neighbors: &[NodeId],
        vectors: &crate::perf_optimizations::ContiguousVectors,
        dimension: usize,
        prefetch_dist: usize,
        best: &mut NodeId,
        best_dist: &mut f32,
    ) -> bool {
        let use_prefetch = should_prefetch(dimension);

        // Prefetch the first batch of neighbor vectors into cache.
        if use_prefetch && neighbors.len() > prefetch_dist {
            Self::prefetch_neighbors(neighbors, vectors, 0, prefetch_dist);
        }

        let mut improved = false;
        for (i, &neighbor) in neighbors.iter().enumerate() {
            // Prefetch upcoming neighbor vectors while processing the current one.
            if use_prefetch && i + prefetch_dist < neighbors.len() {
                Self::prefetch_neighbors(neighbors, vectors, i + prefetch_dist, 1);
            }

            debug_assert!(
                neighbor < vectors.len(),
                "neighbor {neighbor} out of bounds (len {})",
                vectors.len()
            );
            // SAFETY: `get_unchecked` dereferences `neighbor` without bounds checks.
            // - Condition 1: `neighbor < vectors.len()` verified by `debug_assert!` above.
            //   Persisted neighbor IDs are validated `< count` once at load
            //   time (`graph_io::read_node_neighbors`), so this holds in release.
            // SAFETY: Skipping the bounds check avoids a branch in the HNSW hot path.
            let neighbor_vec = unsafe { vectors.get_unchecked(neighbor) };
            let dist = self.distance.distance(query, neighbor_vec);
            if dist < *best_dist {
                *best = neighbor;
                *best_dist = dist;
                improved = true;
            }
        }

        improved
    }

    /// Search a single layer with ef candidates.
    ///
    /// Delegates to [`SearchState`], [`gather_unvisited_neighbors`], and
    /// [`process_batch_results`] to keep each helper under Codacy limits
    /// (CC <= 8, NLOC <= 50).
    ///
    /// F-03 optimization: acquires both vectors and layers read locks once
    /// before the search loop, avoiding ~ef lock acquire/release cycles.
    ///
    /// `stagnation_limit` controls early termination: 0 disables it (use
    /// during index construction to avoid degrading neighbor quality).
    /// For search queries, pass `self.stagnation_limit`.
    ///
    /// `result_limit` controls partial sort optimization: when `Some(k)`,
    /// uses `select_nth_unstable_by` to return only the top-k nearest
    /// results in O(n + k log k) instead of sorting all ef candidates
    /// in O(ef log ef). Pass `None` during construction to get all
    /// candidates sorted (needed for VAMANA neighbor selection).
    #[inline]
    pub(in crate::index::hnsw::native::graph) fn search_layer(
        &self,
        query: &[f32],
        entry_points: &[NodeId],
        ef: usize,
        layer: usize,
        stagnation_limit: usize,
        result_limit: Option<usize>,
    ) -> Vec<(NodeId, f32)> {
        let capacity_hint = self.count.load(Ordering::Relaxed);
        let mut state = SearchState::new(capacity_hint);

        self.with_vectors_and_layers_read(|vectors, layers| {
            let use_prefetch = should_prefetch(vectors.dimension());

            // Initialize entry points
            for &ep in entry_points {
                debug_assert!(
                    ep < vectors.len(),
                    "ep {ep} out of bounds (len {})",
                    vectors.len()
                );
                // SAFETY: `get_unchecked` dereferences `ep` without bounds checks.
                // - Condition 1: `ep < vectors.len()` verified by `debug_assert!` above.
                //   Persisted entry points / neighbor IDs are validated `< count`
                //   once at load time (`graph_io` validation), so this holds in release.
                // SAFETY: Skipping the bounds check avoids a branch in the HNSW hot path.
                let ep_vec = unsafe { vectors.get_unchecked(ep) };
                let dist = self.distance.distance(query, ep_vec);
                state.push_candidate(ep, dist);
            }

            Self::dispatch_layer_search(
                &self.distance,
                query,
                vectors,
                layers,
                &mut state,
                ef,
                layer,
                stagnation_limit,
                use_prefetch,
            );
        });

        state.into_sorted_results(result_limit)
    }

    /// Dispatches layer search to the pipelined or sequential path.
    ///
    /// Pipeline only benefits when the dataset is large enough that neighbor
    /// vectors are frequently evicted from L3 cache between accesses. For
    /// small indices the data stays cache-hot and speculative peek overhead
    /// dominates. Threshold: `>= 10_000` vectors (~30 MB at 768-dim,
    /// platform-agnostic because HNSW random access reduces effective
    /// cache residency).
    ///
    /// Phase 4.3 (#377): both branches honor `use_prefetch`, so datasets
    /// below the pipeline threshold still get intra-gather prefetch when
    /// vectors exceed cache line size. Prefetch is a CPU hint — heap order
    /// and result set remain bit-identical.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn dispatch_layer_search(
        distance: &D,
        query: &[f32],
        vectors: &ContiguousVectors,
        layers: &[Layer],
        state: &mut SearchState,
        ef: usize,
        layer: usize,
        stagnation_limit: usize,
        use_prefetch: bool,
    ) {
        let use_pipeline = use_prefetch && vectors.len() >= 10_000;
        if use_pipeline {
            super::search_pipeline::search_layer_pipelined(
                distance,
                query,
                vectors,
                layers,
                state,
                ef,
                layer,
                stagnation_limit,
                use_prefetch,
            );
        } else {
            Self::search_loop_sequential(
                distance,
                query,
                vectors,
                layers,
                state,
                ef,
                layer,
                stagnation_limit,
                use_prefetch,
            );
        }
    }

    /// Non-pipelined search loop (dataset size below pipeline threshold).
    ///
    /// Sequentially gathers unvisited neighbors, computes distances, and
    /// processes results for each candidate.
    ///
    /// `use_prefetch` enables intra-gather software prefetch of neighbor
    /// vectors (Issue #377, Phase 4.3). Disabled for low-dimension vectors
    /// that already fit in cache — controlled by the caller via
    /// [`should_prefetch`].
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn search_loop_sequential(
        distance: &D,
        query: &[f32],
        vectors: &ContiguousVectors,
        layers: &[Layer],
        state: &mut SearchState,
        ef: usize,
        layer: usize,
        stagnation_limit: usize,
        use_prefetch: bool,
    ) {
        while let Some(Reverse((OrderedFloat(c_dist), c_node))) = state.candidates.pop() {
            if state.should_terminate(c_dist, ef, stagnation_limit) {
                break;
            }

            let improved = layers[layer]
                .with_neighbors(c_node, |neighbors| {
                    let batch = gather_unvisited_neighbors(
                        neighbors,
                        &mut state.visited,
                        vectors,
                        use_prefetch,
                    );
                    if batch.is_empty() {
                        return false;
                    }
                    let vecs: SmallVec<[&[f32]; 32]> = batch.iter().map(|(_, v)| *v).collect();
                    let distances = batch_distance_with_prefetch(distance, query, &vecs);
                    process_batch_results(&batch, &distances, ef, state)
                })
                .unwrap_or(false);

            state.update_stagnation(improved);
        }
    }

    /// Prepares a query vector for search or insertion. Returns `Cow::Borrowed`
    /// for non-cosine metrics (zero-allocation) or `Cow::Owned` with normalized
    /// copy for cosine.
    ///
    /// For cosine, reuses a thread-local buffer to avoid a fresh `Vec<f32>`
    /// allocation on every search call (6 KB saved per 1536-dim query).
    /// The buffer is taken from the thread-local, filled, normalized, and
    /// returned as `Cow::Owned`. When the caller drops the `Cow`, the `Vec`
    /// is freed normally; but the *next* call to `prepare_query` re-seeds
    /// the thread-local if it was left empty, so after warm-up the buffer
    /// allocation is amortized across searches on the same thread.
    #[inline]
    pub(in crate::index::hnsw::native) fn prepare_query<'a>(
        &self,
        query: &'a [f32],
    ) -> Cow<'a, [f32]> {
        if self.distance.is_pre_normalized()
            && self.distance.metric() == crate::DistanceMetric::Cosine
        {
            let mut buf = QUERY_BUF.with(|cell| {
                let mut borrow = cell.borrow_mut();
                if borrow.capacity() == 0 {
                    // First call or after previous Cow was dropped without
                    // returning the buffer — allocate fresh.
                    Vec::with_capacity(query.len())
                } else {
                    std::mem::take(&mut *borrow)
                }
            });
            buf.clear();
            buf.extend_from_slice(query);
            crate::simd_native::normalize_inplace_native(&mut buf);
            Cow::Owned(buf)
        } else {
            Cow::Borrowed(query)
        }
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
}

#[cfg(test)]
mod probe_tests {
    use super::super::super::distance::CpuDistance;
    use super::*;
    use crate::distance::DistanceMetric;

    fn empty_hnsw() -> NativeHnsw<CpuDistance> {
        let dist = CpuDistance::new(DistanceMetric::Euclidean);
        NativeHnsw::new(dist, 16, 200, 0)
    }

    /// Exactly 10 000 vectors must use 1 probe (boundary fix, issue #377).
    ///
    /// Before the fix `count < 10_000` excluded the 10K case, causing the
    /// Balanced preset (ef=160, k=10) benchmark to execute 2 probes.
    #[test]
    fn single_probe_at_exactly_10k() {
        let hnsw = empty_hnsw();
        assert_eq!(hnsw.adaptive_num_probes(10_000, 160, 10), 1);
    }

    #[test]
    fn single_probe_below_10k() {
        let hnsw = empty_hnsw();
        assert_eq!(hnsw.adaptive_num_probes(9_999, 160, 10), 1);
    }

    #[test]
    fn two_probes_above_10k_balanced() {
        let hnsw = empty_hnsw();
        assert_eq!(hnsw.adaptive_num_probes(10_001, 160, 10), 2);
    }

    #[test]
    fn single_probe_for_small_ef() {
        let hnsw = empty_hnsw();
        // ef_search=40 <= max(k*4=40, 64)=64 → single probe at any scale
        assert_eq!(hnsw.adaptive_num_probes(100_000, 40, 10), 1);
    }

    #[test]
    fn four_probes_for_large_ef_at_scale() {
        let hnsw = empty_hnsw();
        assert_eq!(hnsw.adaptive_num_probes(50_000, 1024, 10), 4);
    }

    // =========================================================================
    // Thread-local probe RNG (issue #967)
    // =========================================================================

    /// `next_probe_rng` must never return 0 (XORshift64 invariant) and must
    /// produce at least 64 distinct values over 64 consecutive calls on the
    /// same thread (i.e. no short cycle in any reachable range).
    #[test]
    fn probe_rng_no_zero_and_no_short_cycle() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let v = NativeHnsw::<CpuDistance>::next_probe_rng();
            assert_ne!(v, 0, "XORshift64 must never produce 0");
            seen.insert(v);
        }
        assert_eq!(
            seen.len(),
            64,
            "64 consecutive calls should all be distinct"
        );
    }

    /// Two threads seeded from the same global counter must diverge immediately.
    #[test]
    fn probe_rng_threads_diverge() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let barrier = Arc::new(Barrier::new(2));
        let b1 = Arc::clone(&barrier);
        let b2 = Arc::clone(&barrier);

        let t1 = thread::spawn(move || {
            b1.wait();
            NativeHnsw::<CpuDistance>::next_probe_rng()
        });
        let t2 = thread::spawn(move || {
            b2.wait();
            NativeHnsw::<CpuDistance>::next_probe_rng()
        });

        let v1 = t1.join().expect("thread 1 panicked");
        let v2 = t2.join().expect("thread 2 panicked");
        assert_ne!(
            v1, v2,
            "different threads must start with different RNG values"
        );
    }
}
