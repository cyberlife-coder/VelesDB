//! GPU-accelerated search integration for `NativeHnsw`.
//!
//! Adds `search_gpu()` to `NativeHnsw<D>` which offloads layer-0 BFS
//! expansion to the GPU while keeping upper-layer greedy descent on CPU.
//!
//! Key design properties:
//! - **Per-instance CsrCache**: Each `NativeHnsw` owns its own `gpu_csr_cache`
//!   field, preventing cross-collection contamination. CSR is only rebuilt
//!   when the graph topology changes (insert/delete).
//! - **Multi-entry probing**: Mirrors CPU `adaptive_num_probes` for high-ef
//!   searches, launching GPU traversal from multiple entry points.
//! - **Arc vector snapshot**: Eliminates ~1.5GB per-query memcpy by caching
//!   flat vectors in an `Arc<[f32]>`, refreshed only on count changes.

use super::super::distance::DistanceEngine;
use super::{NativeHnsw, NO_ENTRY_POINT};
use crate::distance::DistanceMetric;
use std::sync::atomic::Ordering;

use crate::gpu::gpu_traversal::GpuTraversalContext;

#[cfg(feature = "gpu")]
impl<D: DistanceEngine> NativeHnsw<D> {
    /// GPU-accelerated search for k nearest neighbors.
    ///
    /// Performs upper-layer greedy descent on CPU (layers 1..max are tiny),
    /// then offloads layer-0 BFS expansion to GPU via the SONG 3-stage pipeline.
    ///
    /// Uses the per-instance `gpu_csr_cache` field for O(1) amortized CSR
    /// access, and multi-entry probing for high-ef recall parity with CPU.
    ///
    /// Returns `None` if GPU is unavailable, the metric is unsupported,
    /// or any GPU error occurs. The caller should fall back to CPU search.
    #[allow(clippy::too_many_lines)] // orchestrates 4 phases (descent, CSR, snapshot, dispatch) — splitting would fragment the GPU call graph; tracked for a follow-up split
    pub fn search_gpu(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        metric: DistanceMetric,
    ) -> Option<Vec<(usize, f32)>> {
        let ep = self.entry_point.load(Ordering::Acquire);
        if ep == NO_ENTRY_POINT {
            return None;
        }

        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return None;
        }

        // Use cached GPU traversal context (pipelines compiled once)
        let ctx = GpuTraversalContext::global()?;

        // Prepare query (normalize for cosine)
        let prepared = self.prepare_query(query);

        // Phase 1: CPU greedy descent through upper layers (tiny, < 1000 nodes)
        let max_layer = self.max_layer.load(Ordering::Relaxed);
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(&prepared, current_ep, layer_idx);
        }

        // Phase 2: Build or reuse cached CSR from layer 0.
        //
        // Uses the per-instance `gpu_csr_cache` field — each NativeHnsw
        // owns its own cache, preventing cross-collection contamination.
        //
        // **Validity signal** — relies solely on the CsrCache dirty flag
        // (`clean_snapshot`), which is aligned with `gpu_snapshot_version`:
        // every mutation that bumps `gpu_snapshot_version` also invalidates
        // the CSR cache through `NativeHnsw::invalidate_gpu_caches`, so
        // "cache clean" ⇔ "snapshot version unchanged" ⇔ "topology
        // unchanged since last build".
        //
        // The historical secondary check `existing.num_nodes == count`
        // (kept on develop until PR-F of #634) was a redundant race
        // guard: it covered the tiny window where `count.fetch_add`
        // completes before `invalidate_gpu_caches` runs on the same
        // mutator thread. Removed as part of the version-counter
        // consolidation — the remaining worst case is that one search
        // serves a CSR with N-1 nodes against an index that just
        // reached N: the newly-inserted node is missed by that single
        // query (same behaviour any delete+insert race has always had)
        // and the next search picks up the fresh rebuild.
        let csr = self.with_layers_read(|layers| {
            if layers.is_empty() {
                return None;
            }
            if let Some(existing) = self.gpu_csr_cache.clean_snapshot() {
                return Some(existing);
            }
            Some(self.gpu_csr_cache.get_or_rebuild(&layers[0], count))
        })?;

        // Phase 3: Get flat vectors for GPU upload via cached snapshot.
        //
        // The snapshot is an `Arc<[f32]>` cached on the NativeHnsw instance.
        // Only refreshed when `gpu_snapshot_version` moves past the recorded
        // build version — any mutation (insert/parallel_insert, future
        // delete) bumps that counter via `invalidate_gpu_caches`, so the
        // cache self-invalidates without depending on `count` alone.
        // Subsequent queries clone the Arc (O(1) pointer bump) instead of
        // copying ~1.5GB of vector data per query.
        let (dimension, vectors_arc) = self.get_or_refresh_vector_snapshot();

        if dimension == 0 || vectors_arc.is_empty() {
            return None;
        }

        let vectors_flat: &[f32] = &vectors_arc;

        // Phase 4: Determine multi-entry probing strategy.
        //
        // Mirrors the CPU adaptive_num_probes pattern for recall parity.
        let num_probes = Self::gpu_adaptive_probes(count, ef_search, k);

        tracing::debug!(
            count,
            dimension,
            csr_edges = csr.total_edges,
            csr_max_degree = csr.max_degree,
            entry_point = current_ep,
            num_probes,
            "GPU search: launching layer-0 traversal"
        );

        if num_probes <= 1 {
            // Single-entry GPU search
            let results = ctx.search_layer0(
                &csr,
                vectors_flat,
                &prepared,
                current_ep,
                k,
                ef_search,
                dimension,
                metric,
            );
            if results.is_empty() {
                None
            } else {
                Some(results)
            }
        } else {
            // Multi-entry GPU search: launch from diversified entry points
            // and merge results (matching CPU search_multi_entry pattern).
            let entry_points = self.diversified_entry_points(current_ep, num_probes, count);

            let mut all_results: Vec<(usize, f32)> = Vec::with_capacity(k * num_probes);
            for &ep_node in &entry_points {
                let results = ctx.search_layer0(
                    &csr,
                    vectors_flat,
                    &prepared,
                    ep_node,
                    k,
                    ef_search,
                    dimension,
                    metric,
                );
                all_results.extend(results);
            }

            if all_results.is_empty() {
                return None;
            }

            // Deduplicate by node ID, then sort by distance.
            //
            // Must sort by node ID first because dedup_by_key only removes
            // *consecutive* duplicates. Sorting by distance first would leave
            // non-adjacent duplicates (same node from different probes)
            // interleaved with other nodes at similar distances.
            all_results.sort_unstable_by_key(|r| r.0);
            all_results.dedup_by_key(|r| r.0);
            all_results.sort_by(|a, b| a.1.total_cmp(&b.1));
            all_results.truncate(k);
            Some(all_results)
        }
    }

    /// Returns cached vector snapshot or refreshes it if the index version
    /// moved past the recorded build version.
    ///
    /// Returns `(dimension, Arc<[f32]>)`. The Arc clone is O(1) on cache hit.
    ///
    /// # Cache validity
    ///
    /// The cache is valid iff the stored `version_at_build` equals
    /// `gpu_snapshot_version` at read time. Every mutation (insert /
    /// parallel_insert, future delete) goes through
    /// [`NativeHnsw::invalidate_gpu_caches`] which bumps the version,
    /// so the cache self-invalidates without relying on callers to
    /// remember to clear the snapshot mutex. A delete-then-insert that
    /// returns to the same count still bumps the version twice and is
    /// therefore detected as stale — the previous count-keyed design
    /// would have served the deleted vector.
    ///
    /// # Lock order
    ///
    /// Acquires `gpu_vectors_snapshot` (rank 5) first, then nests
    /// `vectors` (rank 10) via `with_vectors_read` when the cache is stale.
    /// Instrumented with `record_lock_acquire/release` so the runtime
    /// lock-rank tracker (debug builds) catches any future caller that
    /// inverts the order.
    fn get_or_refresh_vector_snapshot(&self) -> (usize, std::sync::Arc<[f32]>) {
        use super::locking::{record_lock_acquire, record_lock_release, LockRank};

        // Acquire pairs with the Release fetch_add in
        // `invalidate_gpu_caches` so this read observes every prior
        // mutation.
        let current_version = self.gpu_snapshot_version.load(Ordering::Acquire);

        record_lock_acquire(LockRank::GpuVectorsSnapshot);
        let mut snapshot = self.gpu_vectors_snapshot.lock();

        // Check if cached snapshot is still valid against the version.
        if let Some((cached_version, cached_dim, ref cached_vecs)) = *snapshot {
            if cached_version == current_version {
                let hit = (cached_dim, cached_vecs.clone());
                drop(snapshot);
                record_lock_release(LockRank::GpuVectorsSnapshot);
                return hit;
            }
        }

        // Cache miss or stale — refresh. `with_vectors_read` nests
        // `Vectors` (rank 10) inside the snapshot mutex (rank 5), which
        // is the declared lock order.
        let (dim, arc) = self.with_vectors_read(|vectors| {
            let d = vectors.dimension();
            let arc: std::sync::Arc<[f32]> = vectors.as_flat_slice().to_vec().into();
            (d, arc)
        });
        *snapshot = Some((current_version, dim, arc.clone()));
        drop(snapshot);
        record_lock_release(LockRank::GpuVectorsSnapshot);
        (dim, arc)
    }

    /// Adaptive number of GPU entry-point probes.
    ///
    /// Mirrors CPU `adaptive_num_probes` — only probes multiple entry points
    /// for large indices with high ef_search (where recall matters most).
    #[inline]
    fn gpu_adaptive_probes(count: usize, ef_search: usize, k: usize) -> usize {
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

    /// Selects diversified entry points for multi-probe GPU search.
    ///
    /// Uses the primary entry point from greedy descent, then adds
    /// deterministic stride-based points for spatial coverage.
    #[allow(clippy::unused_self)] // kept as a method for symmetry with other NativeHnsw helpers; may grow to consult index state in a follow-up
    fn diversified_entry_points(
        &self,
        primary_ep: usize,
        num_probes: usize,
        count: usize,
    ) -> Vec<usize> {
        let mut eps = Vec::with_capacity(num_probes);
        eps.push(primary_ep);

        let stride = count / num_probes;
        for i in 1..num_probes {
            let candidate = (primary_ep + i * stride) % count;
            if candidate != primary_ep {
                eps.push(candidate);
            }
        }

        eps
    }
}
