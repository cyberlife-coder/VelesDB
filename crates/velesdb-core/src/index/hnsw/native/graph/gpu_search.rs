//! GPU-accelerated search integration for `NativeHnsw`.
//!
//! Adds `search_gpu()` to `NativeHnsw<D>` which offloads layer-0 BFS
//! expansion to the GPU while keeping upper-layer greedy descent on CPU.
//!
//! Optimizations:
//! - **CsrCache**: CSR graph is cached across queries and only rebuilt when
//!   the node count changes (O(1) fast path vs O(N) rebuild).
//! - **Multi-entry probing**: Mirrors CPU `adaptive_num_probes` for high-ef
//!   searches, launching GPU traversal from multiple entry points.

use crate::distance::DistanceMetric;
use super::super::distance::DistanceEngine;
use super::{NativeHnsw, NO_ENTRY_POINT};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use crate::gpu::gpu_csr::{CsrCache, CsrGraph};
use crate::gpu::gpu_traversal::GpuTraversalContext;

/// Global CSR cache — persists across queries, invalidated when node count changes.
fn global_csr_cache() -> &'static CsrCache {
    static CACHE: OnceLock<CsrCache> = OnceLock::new();
    CACHE.get_or_init(CsrCache::new)
}

#[cfg(feature = "gpu")]
impl<D: DistanceEngine> NativeHnsw<D> {
    /// GPU-accelerated search for k nearest neighbors.
    ///
    /// Performs upper-layer greedy descent on CPU (layers 1..max are tiny),
    /// then offloads layer-0 BFS expansion to GPU via the SONG 3-stage pipeline.
    ///
    /// Matches CPU search behavior:
    /// - Uses `adaptive_num_probes` for multi-entry-point probing at high ef_search
    /// - Caches CSR graph across queries (only rebuilds on graph mutations)
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
        // The CsrCache invalidates when we detect the node count has changed
        // since the last rebuild. This avoids the O(N) per-query rebuild cost
        // that Devin flagged — only the first query (or after inserts/deletes)
        // pays the rebuild cost.
        let csr_cache = global_csr_cache();
        let cached_version = csr_cache.version();

        let csr = self.with_layers_read(|layers| {
            if layers.is_empty() {
                return None;
            }
            // Check if count changed since last CSR build; if so, invalidate
            let expected_nodes = count;
            if let Some(existing) = csr_cache.get_if_clean() {
                if existing.num_nodes as usize == expected_nodes {
                    return Some(existing);
                }
                // Node count changed — invalidate and rebuild
                csr_cache.invalidate();
            }
            Some(csr_cache.get_or_rebuild(&layers[0], expected_nodes))
        })?;

        // Phase 3: Extract flat vectors for GPU upload.
        //
        // We hold the vectors lock only during the slice copy. The copy is
        // necessary because the RwLock guard cannot be held across async GPU
        // submission. For 500K+ vectors at 768-dim this is ~1.5GB.
        //
        // Note: The GpuBufferCache infrastructure exists in GpuTraversalContext
        // for future optimization — it would cache GPU-side buffers so this
        // CPU copy only happens on the first query or after data changes.
        let (dimension, vectors_flat) = self.with_vectors_read(|vectors| {
            let dim = vectors.dimension();
            let flat = vectors.as_flat_slice().to_vec();
            (dim, flat)
        });

        if dimension == 0 || vectors_flat.is_empty() {
            return None;
        }

        // Phase 4: Determine multi-entry probing strategy.
        //
        // Mirrors the CPU adaptive_num_probes pattern: for high ef_search,
        // launch GPU traversal from multiple diversified entry points to
        // improve recall on hard queries. For low ef, single probe is fine.
        let num_probes = Self::gpu_adaptive_probes(count, ef_search, k);

        tracing::debug!(
            count,
            dimension,
            csr_edges = csr.total_edges,
            csr_max_degree = csr.max_degree,
            entry_point = current_ep,
            num_probes,
            csr_cache_version = cached_version,
            "GPU search: launching layer-0 traversal"
        );

        if num_probes <= 1 {
            // Single-entry GPU search
            let results = ctx.search_layer0(
                &csr,
                &vectors_flat,
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
                    &vectors_flat,
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
    /// Uses the first entry point from greedy descent, then adds random
    /// nodes that are far from the first entry point to increase coverage.
    fn diversified_entry_points(
        &self,
        _query: &[f32],
        primary_ep: usize,
        num_probes: usize,
        count: usize,
    ) -> Vec<usize> {
        let mut eps = Vec::with_capacity(num_probes);
        eps.push(primary_ep);

        // Add diversified entry points by striding through the node space.
        // This is a simple deterministic strategy that ensures good coverage
        // without requiring distance computations for entry selection.
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
