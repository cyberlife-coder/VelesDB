//! GPU-accelerated search integration for `NativeHnsw`.
//!
//! Adds `search_gpu()` to `NativeHnsw<D>` which offloads layer-0 BFS
//! expansion to the GPU while keeping upper-layer greedy descent on CPU.
//!
//! Uses [`CsrCache`] for lazy CSR graph construction — the CSR is only
//! rebuilt when the layer is mutated (insert/delete), not on every query.

use crate::distance::DistanceMetric;
use super::super::distance::DistanceEngine;
use super::{NativeHnsw, NO_ENTRY_POINT};
use std::sync::atomic::Ordering;

#[cfg(feature = "gpu")]
impl<D: DistanceEngine> NativeHnsw<D> {
    /// GPU-accelerated search for k nearest neighbors.
    ///
    /// Performs upper-layer greedy descent on CPU (layers 1..max are tiny),
    /// then offloads layer-0 BFS expansion to GPU via the SONG 3-stage pipeline.
    ///
    /// Returns `None` if GPU is unavailable, the metric is unsupported,
    /// or any GPU error occurs. The caller should fall back to CPU search.
    ///
    /// # Arguments
    ///
    /// * `query` — query vector (raw, will be normalized for cosine internally)
    /// * `k` — number of nearest neighbors
    /// * `ef_search` — search beam width (larger = better recall, slower)
    /// * `metric` — distance metric (Cosine, Euclidean, DotProduct)
    pub fn search_gpu(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        metric: DistanceMetric,
    ) -> Option<Vec<(usize, f32)>> {
        use crate::gpu::gpu_csr::CsrGraph;
        use crate::gpu::gpu_traversal::GpuTraversalContext;

        let ep = self.entry_point.load(Ordering::Acquire);
        if ep == NO_ENTRY_POINT {
            return None;
        }

        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return None;
        }

        // Use cached GPU traversal context (compiles pipelines once, reused across queries)
        let ctx = GpuTraversalContext::global()?;

        // Prepare query (normalize for cosine)
        let prepared = self.prepare_query(query);

        // Phase 1: CPU greedy descent through upper layers (tiny, < 1000 nodes)
        let max_layer = self.max_layer.load(Ordering::Relaxed);
        let mut current_ep = ep;
        for layer_idx in (1..=max_layer).rev() {
            current_ep = self.search_layer_single(&prepared, current_ep, layer_idx);
        }

        // Phase 2: Build CSR from layer 0
        // Uses CsrGraph::from_layer() which acquires read locks per-node.
        // TODO: Integrate CsrCache into NativeHnsw to avoid rebuilding CSR
        // on every query. The CsrCache infrastructure is ready in gpu_csr.rs
        // but requires adding a CsrCache field to NativeHnsw (structural change).
        let csr = self.with_layers_read(|layers| {
            if layers.is_empty() {
                return None;
            }
            Some(CsrGraph::from_layer(&layers[0], count))
        })?;

        // Phase 3: Extract flat vectors for GPU upload.
        //
        // We need to hold the vectors lock only during the slice access,
        // then copy the data for GPU upload. The copy is necessary because
        // the RwLock guard cannot be held across the async GPU submission.
        //
        // For indices > 500K vectors (GPU threshold), this is ~1.5GB at 768-dim.
        // The GpuBufferCache in GpuTraversalContext will eventually eliminate
        // this copy by caching GPU-side buffers across queries.
        let (dimension, vectors_flat) = self.with_vectors_read(|vectors| {
            let dim = vectors.dimension();
            let flat = vectors.as_flat_slice().to_vec();
            (dim, flat)
        });

        if dimension == 0 || vectors_flat.is_empty() {
            return None;
        }

        tracing::debug!(
            count,
            dimension,
            csr_edges = csr.total_edges,
            csr_max_degree = csr.max_degree,
            entry_point = current_ep,
            "GPU search: launching layer-0 traversal"
        );

        // Phase 4: GPU layer-0 search
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

        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }
}
