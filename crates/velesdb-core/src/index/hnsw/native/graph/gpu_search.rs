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

use crate::distance::DistanceMetric;
use super::super::distance::DistanceEngine;
use super::{NativeHnsw, NO_ENTRY_POINT};
use std::sync::atomic::Ordering;

use crate::gpu::gpu_csr::CsrGraph;
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
        // The CsrCache only rebuilds when invalidated (insert/delete) or
        // when the node count changes.
        let csr = self.with_layers_read(|layers| {
            if layers.is_empty() {
                return None;
            }
            // Check if cached CSR is still valid for this graph
            if let Some(existing) = self.gpu_csr_cache.get_if_clean() {
                if existing.num_nodes as usize == count {
                    return Some(existing);
                }
                // Node count changed — invalidate and rebuild
                self.gpu_csr_cache.invalidate();
            }
            Some(self.gpu_csr_cache.get_or_rebuild(&layers[0], count))
        })?;

        // Phase 3: Get flat vectors for GPU upload via cached snapshot.
        //
        // The snapshot is an `Arc<[f32]>` cached on the NativeHnsw instance.
        // Only refreshed when the vector count changes (insert/delete).
        // Subsequent queries clone the Arc (O(1) pointer bump) instead of
        // copying ~1.5GB of vector data per query.
        let (dimension, vectors_arc) = {
            let mut snapshot = self.gpu_vectors_snapshot.lock();
            if let Some((cached_count, cached_dim, ref cached_vecs)) = *snapshot {
                if cached_count == count {
                    (cached_dim, cached_vecs.clone())
                } else {
                    // Count changed — refresh snapshot
                    let (dim, arc) = self.with_vectors_read(|vectors| {
                        let d = vectors.dimension();
                        let arc: std::sync::Arc<[f32]> =
                            vectors.as_flat_slice().to_vec().into();
                        (d, arc)
                    });
                    *snapshot = Some((count, dim, arc.clone()));
                    (dim, arc)
                }
            } else {
                // First call — create snapshot
                let (dim, arc) = self.with_vectors_read(|vectors| {
                    let d = vectors.dimension();
                    let arc: std::sync::Arc<[f32]> =
                        vectors.as_flat_slice().to_vec().into();
                    (d, arc)
                });
                *snapshot = Some((count, dim, arc.clone()));
                (dim, arc)
            }
        };

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
            if results.is_empty() { None } else { Some(results) }
        } else {
            // Multi-entry GPU search: launch from diversified entry points
            // and merge results (matching CPU search_multi_entry pattern).
            let entry_points = self.diversified_entry_points(
                &prepared, current_ep, num_probes, count,
            );

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

            // Deduplicate and sort by distance
            all_results.sort_by(|a, b| a.1.total_cmp(&b.1));
            all_results.dedup_by_key(|r| r.0);
            all_results.truncate(k);
            Some(all_results)
        }
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
    fn diversified_entry_points(
        &self,
        _query: &[f32],
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
