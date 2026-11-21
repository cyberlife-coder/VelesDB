//! Native graph access and device routing for each storage backend.

use super::{CachedSimdDistance, HnswBackend, NativeHnsw, NativeHnswInner};

impl NativeHnswInner {
    /// The f32 graph whose arena and metric all three backends use for their
    /// final row score, including the exact rerank after SQ8/RaBitQ traversal.
    pub(in crate::index::hnsw) fn scoring_graph(&self) -> &NativeHnsw<CachedSimdDistance> {
        match &self.backend {
            HnswBackend::Standard(graph) => graph,
            HnswBackend::RaBitQ(codec) => &codec.inner,
            HnswBackend::Sq8(codec) => &codec.inner,
        }
    }

    /// Shared device routing for [`Self::search_auto`] and [`Self::search_resumable`].
    /// Attempts device traversal when the Standard backend crosses the GPU threshold. `None` means "take the CPU
    /// path" — either the feature is off, the backend/scale does not qualify,
    /// or the GPU attempt failed and search falls through.
    ///
    /// `query.len()` is authoritative for the index dimension: a query of
    /// wrong length would fail distance evaluation anyway.
    // Both allows have the same single reason: this body is entirely inside
    // `#[cfg(feature = "gpu")]`, so with the feature off nothing in the
    // signature is read -- neither the parameters nor the receiver. The
    // receiver's allow was missing, which made
    // `cargo clippy -p velesdb-core --lib --features persistence -- -D warnings
    // -D clippy::pedantic` fail on a clean tree. The workspace allows
    // `unused_self`, but a `-D clippy::pedantic` on the command line overrides
    // it, so the attribute is what that strict form needs; CI never sees the
    // gap because it lints one feature set, and that set always has `gpu`.
    #[allow(unused_variables)] // Reason: parameters unused when `gpu` is off
    #[allow(clippy::unused_self)] // Reason: receiver unused when `gpu` is off
    pub(super) fn try_gpu_route(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Option<Vec<(usize, f32)>> {
        #[cfg(feature = "gpu")]
        if self.gpu_traversal_eligible(query.len()) {
            return self.search_gpu(query, k, ef_search);
        }
        None
    }

    #[cfg(feature = "gpu")]
    pub(in crate::index::hnsw) fn gpu_traversal_eligible(&self, dimension: usize) -> bool {
        matches!(&self.backend, HnswBackend::Standard(graph) if crate::gpu::should_traverse_gpu(graph.len(), dimension))
    }

    /// Attempts GPU-accelerated search on the Standard backend.
    ///
    /// Returns `None` if GPU is unavailable, the metric is unsupported,
    /// or any GPU operation fails. The caller should fall back to CPU search.
    #[cfg(feature = "gpu")]
    fn search_gpu(&self, query: &[f32], k: usize, ef_search: usize) -> Option<Vec<(usize, f32)>> {
        let hnsw = match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw,
            HnswBackend::RaBitQ(_) | HnswBackend::Sq8(_) => return None,
        };

        hnsw.search_gpu(query, k, ef_search, self.metric)
    }
}
