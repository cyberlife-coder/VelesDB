//! HNSW search operations.

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::super::ordered_float::OrderedFloat;
use super::NativeHnsw;
use rustc_hash::FxHashSet;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BinaryHeap;
use std::sync::atomic::Ordering;

/// Returns whether prefetch hints should be used for vectors of the given dimension.
///
/// Threshold: vector must span at least 2 cache lines (128 bytes = 32 f32 elements).
/// Below this, prefetch overhead exceeds the benefit.
#[inline]
fn should_prefetch(dimension: usize) -> bool {
    let vector_bytes = dimension * std::mem::size_of::<f32>();
    vector_bytes >= 2 * crate::simd_native::L2_CACHE_LINE_BYTES
}

// Thread-local pool of reusable visited sets to avoid repeated allocations
// during batch HNSW searches. Each thread keeps up to 4 sets.
thread_local! {
    static VISITED_POOL: RefCell<Vec<FxHashSet<usize>>> = const { RefCell::new(Vec::new()) };
}

/// Borrows a visited set from the thread-local pool, or creates a new one.
#[inline]
fn acquire_visited_set() -> FxHashSet<usize> {
    VISITED_POOL.with(|pool| pool.borrow_mut().pop().unwrap_or_default())
}

/// Returns a visited set to the thread-local pool after clearing it.
/// Keeps at most 4 sets per thread to bound memory usage.
#[inline]
fn release_visited_set(mut set: FxHashSet<usize>) {
    set.clear();
    VISITED_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < 4 {
            pool.push(set);
        }
    });
}

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Searches for k nearest neighbors.
    #[inline]
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        let prepared_query = self.prepare_query(query);
        let query: &[f32] = &prepared_query;
        let entry_point = *self.entry_point.read();
        let Some(ep) = entry_point else {
            return Vec::new();
        };

        let max_layer = self.max_layer.load(Ordering::Relaxed);

        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        let count = self.count.load(Ordering::Relaxed);
        let probes = self.adaptive_num_probes(count, ef_search, k);

        if probes > 1 {
            return self.search_multi_entry(query, k, ef_search, probes);
        }

        let candidates =
            self.search_layer(query, vec![current_ep], ef_search, 0, self.stagnation_limit);
        candidates.into_iter().take(k).collect()
    }

    /// Adaptive number of entry-point probes for high-recall searches.
    #[inline]
    fn adaptive_num_probes(&self, count: usize, ef_search: usize, k: usize) -> usize {
        if count < 10_000 || ef_search <= (k * 4).max(64) {
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
    #[must_use]
    pub fn search_multi_entry(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        num_probes: usize,
    ) -> Vec<(NodeId, f32)> {
        let prepared_query = self.prepare_query(query);
        let query: &[f32] = &prepared_query;
        let entry_point = *self.entry_point.read();
        let Some(ep) = entry_point else {
            return Vec::new();
        };

        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return Vec::new();
        }

        let max_layer = self.max_layer.load(Ordering::Relaxed);

        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        let mut entry_points = vec![current_ep];
        if num_probes > 1 && count > 10 {
            for _ in 1..num_probes.min(4) {
                let old_state = self
                    .rng_state
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |mut state| {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        Some(state)
                    })
                    .unwrap_or_else(|s| s);

                let mut state = old_state;
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;

                let random_id = (state as usize) % count;
                if !entry_points.contains(&random_id) {
                    entry_points.push(random_id);
                }
            }
        }

        let candidates =
            self.search_layer(query, entry_points, ef_search, 0, self.stagnation_limit);
        candidates.into_iter().take(k).collect()
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
            // SAFETY: `entry` is a valid node_id from entry_point (always a
            // successfully inserted node).
            // - Condition 1: entry < vectors.len() (from a prior successful insert).
            // Reason: Hot-path greedy descent — bounds check eliminated.
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

            // SAFETY: neighbor is a valid node_id from the graph's
            // neighbor list, only containing IDs of inserted nodes.
            // - Condition 1: neighbor < vectors.len().
            // Reason: Inner loop of greedy descent — bounds check eliminated.
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
    /// F-03 optimization: acquires both vectors and layers read locks once
    /// before the search loop, avoiding ~ef lock acquire/release cycles.
    ///
    /// `stagnation_limit` controls early termination: 0 disables it (use
    /// during index construction to avoid degrading neighbor quality).
    /// For search queries, pass `self.stagnation_limit`.
    #[inline]
    pub(in crate::index::hnsw::native::graph) fn search_layer(
        &self,
        query: &[f32],
        entry_points: Vec<NodeId>,
        ef: usize,
        layer: usize,
        stagnation_limit: usize,
    ) -> Vec<(NodeId, f32)> {
        use std::cmp::Reverse;

        let mut visited = acquire_visited_set();
        let mut candidates: BinaryHeap<Reverse<(OrderedFloat, NodeId)>> = BinaryHeap::new();
        let mut results: BinaryHeap<(OrderedFloat, NodeId)> = BinaryHeap::new();

        self.with_vectors_and_layers_read(|vectors, layers| {
            let dimension = vectors.dimension();
            let prefetch_distance = crate::simd_native::calculate_prefetch_distance(dimension);
            let mut stagnation_count: usize = 0;

            for ep in entry_points {
                // SAFETY: ep is a valid node_id from entry_point or random probe,
                // always IDs of successfully inserted nodes.
                // - Condition 1: ep < vectors.len().
                // Reason: Entry-point initialization in search hot path.
                let ep_vec = unsafe { vectors.get_unchecked(ep) };
                let dist = self.distance.distance(query, ep_vec);
                candidates.push(Reverse((OrderedFloat(dist), ep)));
                results.push((OrderedFloat(dist), ep));
                visited.insert(ep);
            }

            while let Some(Reverse((OrderedFloat(c_dist), c_node))) = candidates.pop() {
                let furthest_dist = results.peek().map_or(f32::MAX, |r| r.0 .0);

                if c_dist > furthest_dist && results.len() >= ef {
                    break;
                }

                // Early termination on stagnation: if many consecutive candidates
                // fail to improve the result set, the search region is exhausted.
                if stagnation_limit > 0 && stagnation_count >= stagnation_limit {
                    break;
                }

                let improved = layers[layer]
                    .with_neighbors(c_node, |neighbors| {
                        let mut improved_this_round = false;

                        if should_prefetch(dimension) && neighbors.len() > prefetch_distance {
                            for &neighbor_id in neighbors.iter().take(prefetch_distance) {
                                if neighbor_id < vectors.len() {
                                    vectors.prefetch(neighbor_id);
                                }
                            }
                        }

                        for (i, neighbor) in neighbors.iter().enumerate() {
                            if should_prefetch(dimension) && i + prefetch_distance < neighbors.len()
                            {
                                let prefetch_id = neighbors[i + prefetch_distance];
                                if prefetch_id < vectors.len() {
                                    vectors.prefetch(prefetch_id);
                                }
                            }

                            if visited.insert(*neighbor) {
                                // SAFETY: *neighbor is a valid node_id from the graph's
                                // neighbor list, only containing IDs of inserted nodes.
                                // - Condition 1: *neighbor < vectors.len().
                                // Reason: Inner search loop — bounds check eliminated.
                                let n_vec = unsafe { vectors.get_unchecked(*neighbor) };
                                let dist = self.distance.distance(query, n_vec);
                                let furthest = results.peek().map_or(f32::MAX, |r| r.0 .0);

                                if dist < furthest || results.len() < ef {
                                    candidates.push(Reverse((OrderedFloat(dist), *neighbor)));
                                    results.push((OrderedFloat(dist), *neighbor));

                                    if results.len() > ef {
                                        results.pop();
                                    }
                                    improved_this_round = true;
                                }
                            }
                        }

                        improved_this_round
                    })
                    .unwrap_or(false);

                if improved {
                    stagnation_count = 0;
                } else {
                    stagnation_count += 1;
                }
            }
        });

        release_visited_set(visited);

        let mut result_vec: Vec<(NodeId, f32)> =
            results.into_iter().map(|(d, n)| (n, d.0)).collect();
        result_vec.sort_by(|a, b| a.1.total_cmp(&b.1));
        result_vec
    }

    /// Prepares the query vector for search. Returns `Cow::Borrowed` for non-cosine
    /// metrics (zero-allocation) or `Cow::Owned` with normalized copy for cosine.
    #[inline]
    fn prepare_query<'a>(&self, query: &'a [f32]) -> Cow<'a, [f32]> {
        if self.distance.is_pre_normalized()
            && self.distance.metric() == crate::DistanceMetric::Cosine
        {
            let mut prepared = query.to_vec();
            crate::simd_native::normalize_inplace_native(&mut prepared);
            Cow::Owned(prepared)
        } else {
            Cow::Borrowed(query)
        }
    }
}
