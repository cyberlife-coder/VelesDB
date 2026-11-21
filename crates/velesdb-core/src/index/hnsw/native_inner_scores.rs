//! Score conversion and access to the vector arena of each native backend.

use super::{CachedSimdDistance, HnswBackend, NativeHnsw, NativeHnswInner};

impl NativeHnswInner {
    /// Transforms raw HNSW distance to the appropriate score.
    ///
    /// For **Standard** backend: applies metric-specific transform.
    /// For `RaBitQ` backend: identity (scores already transformed).
    #[inline]
    #[must_use]
    pub fn transform_score(&self, raw_distance: f32) -> f32 {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.transform_score(raw_distance),
            HnswBackend::RaBitQ(_) | HnswBackend::Sq8(_) => raw_distance,
        }
    }

    /// The f32 graph whose arena and metric all three backends use for their
    /// final row score, including the exact rerank after SQ8/RaBitQ traversal.
    #[cfg(feature = "persistence")]
    pub(in crate::index::hnsw) fn scoring_graph(&self) -> &NativeHnsw<CachedSimdDistance> {
        match &self.backend {
            HnswBackend::Standard(graph) => graph,
            HnswBackend::RaBitQ(codec) => &codec.inner,
            HnswBackend::Sq8(codec) => &codec.inner,
        }
    }

    /// Returns the number of elements in the index.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.len(),
            HnswBackend::RaBitQ(rabitq) => rabitq.len(),
            HnswBackend::Sq8(sq8) => sq8.len(),
        }
    }

    /// Computes the raw distance between two vectors.
    #[inline]
    #[must_use]
    pub fn compute_distance(&self, a: &[f32], b: &[f32]) -> f32 {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.compute_distance(a, b),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.compute_distance(a, b),
            HnswBackend::Sq8(sq8) => sq8.inner.compute_distance(a, b),
        }
    }

    /// Executes a closure with zero-copy access to the contiguous vector storage.
    ///
    /// Returns `R::default()` if vector storage is not yet initialized.
    #[inline]
    pub fn with_contiguous_vectors<R: Default>(
        &self,
        f: impl FnOnce(&crate::perf_optimizations::ContiguousVectors) -> R,
    ) -> R {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.with_vectors_read(f),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.with_vectors_read(f),
            HnswBackend::Sq8(sq8) => sq8.inner.with_vectors_read(f),
        }
    }

    /// Executes a closure with read access to the contiguous vector storage.
    ///
    /// Alias for [`with_contiguous_vectors`](Self::with_contiguous_vectors)
    /// with explicit read semantics for clarity at call sites.
    #[cfg(test)]
    #[inline]
    pub fn with_contiguous_vectors_read<R: Default>(
        &self,
        f: impl FnOnce(&crate::perf_optimizations::ContiguousVectors) -> R,
    ) -> R {
        self.with_contiguous_vectors(f)
    }
}
