//! Native HNSW inner implementation - replaces `hnsw_rs` dependency.
//!
//! This module provides `NativeHnswInner`, a drop-in replacement for `HnswInner`
//! that uses our native HNSW implementation instead of the `hnsw_rs` crate.
//!
//! Supports two backends via [`HnswBackend`]:
//! - **Standard**: Full f32 distances (`NativeHnsw`)
//! - **`RaBitQ`**: Binary traversal + f32 re-ranking (`RaBitQPrecisionHnsw`)

// Temporarily allow dead_code until integration into HnswIndex
#![allow(dead_code)]
#![allow(clippy::cast_precision_loss)]

use super::native::rabitq_precision::RaBitQPrecisionHnsw;
use super::native::{CachedSimdDistance, NativeHnsw, NativeNeighbour, DEFAULT_ALPHA};
use crate::distance::DistanceMetric;
use std::path::Path;

/// Backend selector for the native HNSW index.
///
/// `Standard` uses full f32 distances. `RaBitQ` uses binary graph traversal
/// (32x compression) with f32 re-ranking for final results.
// Reason: `Standard` (272 B) is the hot path — boxing it would add pointer
// indirection on every search call. `RaBitQ` is boxed intentionally to avoid
// inflating `Standard`-mode layout across cache lines.
#[allow(clippy::large_enum_variant)]
enum HnswBackend {
    /// Standard f32 distance backend.
    Standard(NativeHnsw<CachedSimdDistance>),
    /// `RaBitQ` binary traversal + f32 re-ranking backend.
    ///
    /// Boxed to keep the enum size equal to `NativeHnsw` (~64 bytes).
    /// `RaBitQPrecisionHnsw` is ~250 bytes (3 locks + buffers); storing it
    /// inline would push `Standard`-mode hot fields across cache lines.
    RaBitQ(Box<RaBitQPrecisionHnsw<CachedSimdDistance>>),
}

/// Native HNSW index wrapper to handle different distance metrics and backends.
///
/// This is the native equivalent of `HnswInner`, using our own HNSW implementation
/// instead of `hnsw_rs`. It provides the same API for seamless integration.
pub struct NativeHnswInner {
    /// The underlying HNSW backend (standard or `RaBitQ`).
    backend: HnswBackend,
    /// The distance metric used.
    metric: DistanceMetric,
}

impl NativeHnswInner {
    /// Creates a new `NativeHnswInner` with the specified metric and parameters.
    ///
    /// Uses the default VAMANA alpha (1.2) for neighbor diversification.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new(
        metric: DistanceMetric,
        max_connections: usize,
        max_elements: usize,
        ef_construction: usize,
        dimension: usize,
    ) -> crate::error::Result<Self> {
        Self::new_with_options(
            metric,
            max_connections,
            max_elements,
            ef_construction,
            dimension,
            crate::StorageMode::Full,
            DEFAULT_ALPHA,
        )
    }

    /// Creates a new `NativeHnswInner` with a specific storage mode.
    ///
    /// When `storage_mode` is [`StorageMode::RaBitQ`], the backend uses binary
    /// graph traversal for 32x bandwidth reduction during search.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new_with_storage_mode(
        metric: DistanceMetric,
        max_connections: usize,
        max_elements: usize,
        ef_construction: usize,
        dimension: usize,
        storage_mode: crate::StorageMode,
    ) -> crate::error::Result<Self> {
        Self::new_with_options(
            metric,
            max_connections,
            max_elements,
            ef_construction,
            dimension,
            storage_mode,
            DEFAULT_ALPHA,
        )
    }

    /// Creates a new `NativeHnswInner` with full configuration options.
    ///
    /// This is the canonical constructor; all other `new*` methods delegate here.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_options(
        metric: DistanceMetric,
        max_connections: usize,
        max_elements: usize,
        ef_construction: usize,
        dimension: usize,
        storage_mode: crate::StorageMode,
        alpha: f32,
    ) -> crate::error::Result<Self> {
        let backend = if matches!(storage_mode, crate::StorageMode::RaBitQ) {
            let distance = CachedSimdDistance::new(metric, dimension);
            let rabitq = RaBitQPrecisionHnsw::new_with_alpha(
                distance,
                dimension,
                max_connections,
                ef_construction,
                max_elements,
                alpha,
            )?;
            HnswBackend::RaBitQ(Box::new(rabitq))
        } else {
            let distance = CachedSimdDistance::new(metric, dimension);
            let inner = if dimension > 0 {
                NativeHnsw::new_with_dimension_and_alpha(
                    distance,
                    max_connections,
                    ef_construction,
                    max_elements,
                    dimension,
                    alpha,
                )?
            } else {
                NativeHnsw::with_alpha(
                    distance,
                    max_connections,
                    ef_construction,
                    max_elements,
                    alpha,
                )
            };
            HnswBackend::Standard(inner)
        };

        Ok(Self { backend, metric })
    }

    /// Returns the storage mode for this backend.
    #[must_use]
    pub fn storage_mode(&self) -> crate::StorageMode {
        match &self.backend {
            HnswBackend::Standard(_) => crate::StorageMode::Full,
            HnswBackend::RaBitQ(_) => crate::StorageMode::RaBitQ,
        }
    }
}

// ============================================================================
// Search methods
// ============================================================================

impl NativeHnswInner {
    /// Searches the HNSW graph and returns `(node_id, distance)` tuples.
    ///
    /// For **Standard** backend: returns raw distances (caller must call
    /// [`transform_score`](Self::transform_score)).
    ///
    /// For `RaBitQ` backend: returns pre-transformed scores (caller's
    /// `transform_score` is a no-op identity).
    #[inline]
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(usize, f32)> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.search(query, k, ef_search),
            HnswBackend::RaBitQ(rabitq) => rabitq.search(query, k, ef_search),
        }
    }

    /// Searches the HNSW graph and returns results as `NativeNeighbour` structs.
    #[inline]
    #[must_use]
    pub fn search_neighbours(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<NativeNeighbour> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.search_neighbours(query, k, ef_search),
            HnswBackend::RaBitQ(rabitq) => rabitq
                .search(query, k, ef_search)
                .into_iter()
                .map(|(id, dist)| NativeNeighbour {
                    d_id: id,
                    distance: dist,
                })
                .collect(),
        }
    }
}

// ============================================================================
// Insert methods
// ============================================================================

impl NativeHnswInner {
    /// Inserts a single vector into the HNSW graph.
    ///
    /// The caller supplies `(vector, expected_idx)` where `expected_idx` is the
    /// internal index pre-registered in `ShardedMappings`.
    ///
    /// # Errors
    ///
    /// Returns an error if allocation, insertion, or ID-mapping consistency fails.
    pub fn insert(&self, data: (&[f32], usize)) -> crate::error::Result<usize> {
        let (vector, expected_idx) = data;
        let assigned_id = match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.insert(vector)?,
            HnswBackend::RaBitQ(rabitq) => rabitq.insert(vector)?,
        };
        if assigned_id != expected_idx {
            tracing::warn!(
                "NativeHnsw node_id mismatch: expected {expected_idx}, got {assigned_id} \
                 — mapping may be desynchronised under concurrent inserts"
            );
        }
        Ok(assigned_id)
    }

    /// Parallel batch insert into the HNSW graph.
    ///
    /// # Errors
    ///
    /// Returns an error if any insertion fails.
    pub fn parallel_insert(&self, data: &[(&[f32], usize)]) -> crate::error::Result<Vec<usize>> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.parallel_insert(data),
            // RaBitQ: insert sequentially to maintain RaBitQ store consistency
            HnswBackend::RaBitQ(_) => {
                let mut ids = Vec::with_capacity(data.len());
                for &(vector, expected_idx) in data {
                    ids.push(self.insert((vector, expected_idx))?);
                }
                Ok(ids)
            }
        }
    }

    /// Sets the index to searching mode after bulk insertions.
    pub fn set_searching_mode(&mut self, mode: bool) {
        match &mut self.backend {
            HnswBackend::Standard(hnsw) => hnsw.set_searching_mode(mode),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.set_searching_mode(mode),
        }
    }

    /// Reorders graph nodes in BFS traversal order for improved cache locality.
    ///
    /// After reordering, vectors that are close in the graph are also close
    /// in memory, reducing cache misses during search traversal.
    ///
    /// Skips reordering for small indices (< 1000 vectors) where the entire
    /// working set fits in L2 cache.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage reordering fails.
    pub fn reorder_for_locality(&self) -> crate::error::Result<()> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.reorder_for_locality(),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.reorder_for_locality(),
        }
    }
}

// ============================================================================
// Persistence methods
// ============================================================================

impl NativeHnswInner {
    /// Dumps the HNSW graph to files for persistence.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail.
    pub fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.file_dump(path, basename),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.file_dump(path, basename),
        }
    }

    /// Loads the HNSW graph from files.
    ///
    /// When `storage_mode` is [`StorageMode::RaBitQ`], wraps the loaded graph
    /// in a `RaBitQPrecisionHnsw`. The quantizer trains lazily after enough
    /// new vectors are inserted.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail or data is corrupted.
    pub fn file_load(
        path: &Path,
        basename: &str,
        metric: DistanceMetric,
        dimension: usize,
    ) -> std::io::Result<Self> {
        let distance = CachedSimdDistance::new(metric, dimension);
        let inner = NativeHnsw::file_load(path, basename, distance)?;

        Ok(Self {
            backend: HnswBackend::Standard(inner),
            metric,
        })
    }

    /// Loads the HNSW graph with a specific storage mode.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail or data is corrupted.
    pub fn file_load_with_storage_mode(
        path: &Path,
        basename: &str,
        metric: DistanceMetric,
        dimension: usize,
        storage_mode: crate::StorageMode,
    ) -> std::io::Result<Self> {
        let distance = CachedSimdDistance::new(metric, dimension);
        let inner = NativeHnsw::file_load(path, basename, distance)?;

        let backend = if matches!(storage_mode, crate::StorageMode::RaBitQ) {
            // Wrap loaded graph in RaBitQ backend.
            // The quantizer is NOT trained yet — it trains lazily from new inserts.
            let distance = CachedSimdDistance::new(metric, dimension);
            let rabitq = RaBitQPrecisionHnsw::from_inner(inner, distance, dimension);
            HnswBackend::RaBitQ(Box::new(rabitq))
        } else {
            HnswBackend::Standard(inner)
        };

        Ok(Self { backend, metric })
    }
}

// ============================================================================
// Score and distance methods
// ============================================================================

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
            HnswBackend::RaBitQ(_) => raw_distance,
        }
    }

    /// Returns the number of elements in the index.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.len(),
            HnswBackend::RaBitQ(rabitq) => rabitq.len(),
        }
    }

    /// Returns true if the index is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.is_empty(),
            HnswBackend::RaBitQ(rabitq) => rabitq.is_empty(),
        }
    }

    /// Returns the distance metric used by this index.
    #[inline]
    #[must_use]
    pub fn metric(&self) -> DistanceMetric {
        self.metric
    }

    /// Computes the raw distance between two vectors.
    #[inline]
    #[must_use]
    pub fn compute_distance(&self, a: &[f32], b: &[f32]) -> f32 {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.compute_distance(a, b),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.compute_distance(a, b),
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
        }
    }
}

// ============================================================================
// Send + Sync for thread safety
// ============================================================================

// SAFETY: `NativeHnswInner` is `Send` because ownership transfer preserves invariants.
// - Condition 1: Internal mutability is synchronized via `parking_lot::RwLock`/atomics.
// - Condition 2: No thread-affine resources are stored in the wrapper.
// Reason: Moving the index wrapper between threads is sound.
unsafe impl Send for NativeHnswInner {}
// SAFETY: `NativeHnswInner` is `Sync` because shared references are concurrency-safe.
// - Condition 1: Concurrent access to mutable graph state is lock/atomic protected.
// - Condition 2: Exposed APIs do not bypass synchronization primitives.
// Reason: `&NativeHnswInner` can be shared safely across threads.
unsafe impl Sync for NativeHnswInner {}

// ============================================================================
// Tests moved to native_inner_tests.rs per project rules
