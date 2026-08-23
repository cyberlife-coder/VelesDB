//! Native HNSW inner implementation - replaces `hnsw_rs` dependency.
//!
//! This module provides `NativeHnswInner`, a drop-in replacement for `HnswInner`
//! that uses our native HNSW implementation instead of the `hnsw_rs` crate.
//!
//! Supports three backends via [`HnswBackend`]:
//! - **Standard**: Full f32 distances (`NativeHnsw`)
//! - **`RaBitQ`**: Binary traversal + f32 re-ranking (`RaBitQPrecisionHnsw`)
//! - **SQ8**: Int8 traversal + f32 re-ranking (`Sq8PrecisionHnsw`)

#![allow(clippy::cast_precision_loss)]

use super::native::rabitq_precision::RaBitQPrecisionHnsw;
use super::native::sq8_precision::Sq8PrecisionHnsw;
use super::native::{CachedSimdDistance, NativeHnsw, NativeNeighbour, ResumableSearch};
use crate::distance::DistanceMetric;
use std::path::Path;

/// Opaque resume handle for an escalatable CPU search on the Standard backend.
///
/// Wraps the graph-level [`ResumableSearch`] so callers above the backend
/// dispatch hold it without seeing graph internals. Dropping it releases the
/// pooled traversal structures.
pub struct SearchResume(ResumableSearch);

/// Backend selector for the native HNSW index.
///
/// `Standard` uses full f32 distances. `RaBitQ` uses binary graph traversal
/// (32x compression) with f32 re-ranking; `Sq8` uses int8 traversal (4x
/// bandwidth reduction) with f32 re-ranking.
// SAFETY: `Standard` (272 B) is the hot path — boxing it would add pointer
// indirection on every search call. The quantized backends are boxed
// intentionally to avoid inflating `Standard`-mode layout across cache lines.
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
    /// SQ8 int8 traversal + f32 re-ranking backend (boxed, same rationale).
    Sq8(Box<Sq8PrecisionHnsw<CachedSimdDistance>>),
}

/// Native HNSW index wrapper to handle different distance metrics and backends.
///
/// This is the native equivalent of `HnswInner`, using our own HNSW implementation
/// instead of `hnsw_rs`. It provides the same API for seamless integration.
pub struct NativeHnswInner {
    /// The underlying HNSW backend (Standard, `RaBitQ`, or SQ8).
    backend: HnswBackend,
    /// The distance metric used.
    #[allow(dead_code)] // Reason: Exposed via `metric()` accessor — API surface for callers
    metric: DistanceMetric,
}

/// Everything needed to build a [`NativeHnswInner`].
///
/// A struct rather than an eighth positional parameter: the constructor had
/// already earned a `too_many_arguments` allow at seven, and four of them are
/// `usize`, so a transposed pair compiles and misbehaves silently. Named
/// fields make that a compile error instead.
pub(crate) struct InnerBuild<'a> {
    /// Distance metric the graph is built for.
    pub metric: DistanceMetric,
    /// M — connections per node above layer 0.
    pub max_connections: usize,
    /// Capacity hint for layer pre-allocation.
    pub max_elements: usize,
    /// Beam width during construction.
    pub ef_construction: usize,
    /// Vector dimension; `0` defers arena allocation to the first insert.
    pub dimension: usize,
    /// Selects the backend, and with it whether a mapped arena applies.
    pub storage_mode: crate::StorageMode,
    /// VAMANA diversification factor.
    pub alpha: f32,
    /// Directory for the f32 arena file, when it should live on disk.
    ///
    /// Honoured only by the quantized backends — see [`NativeHnswInner::build`]
    /// for why a `Full` graph must keep its arena in memory. `None` anywhere
    /// else, including for a collection with no directory of its own.
    pub arena_dir: Option<&'a std::path::Path>,
}

impl NativeHnswInner {
    /// Creates a new `NativeHnswInner` with full configuration options.
    ///
    /// This is the canonical constructor; all other `new*` methods delegate here.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new_with_options(
        metric: DistanceMetric,
        max_connections: usize,
        max_elements: usize,
        ef_construction: usize,
        dimension: usize,
        storage_mode: crate::StorageMode,
        alpha: f32,
    ) -> crate::error::Result<Self> {
        Self::build(&InnerBuild {
            metric,
            max_connections,
            max_elements,
            ef_construction,
            dimension,
            storage_mode,
            alpha,
            arena_dir: None,
        })
    }

    /// Builds a backend from a fully-specified [`InnerBuild`].
    ///
    /// The canonical constructor; `new_with_options` is the positional
    /// shorthand for the common case of a graph with no arena directory.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation or mapping fails.
    pub(crate) fn build(opts: &InnerBuild<'_>) -> crate::error::Result<Self> {
        let distance = CachedSimdDistance::new_prenormalized(opts.metric, opts.dimension);
        // `arena_dir` is honoured for every mode here, on purpose. Whether a
        // mapped arena *suits* a graph depends on how that graph will be
        // read, which only the caller knows: vacuum builds a `Full` graph it
        // is about to promote to a quantized one, and that graph must be
        // mapped even though its mode says otherwise. Deciding it here would
        // make that case unreachable, so the policy lives with the callers —
        // see `HnswIndex::with_params_in_dir`.
        let backend = match opts.storage_mode {
            crate::StorageMode::RaBitQ => {
                HnswBackend::RaBitQ(Box::new(RaBitQPrecisionHnsw::with_optional_arena_dir(
                    distance,
                    opts.dimension,
                    opts.max_connections,
                    opts.ef_construction,
                    opts.max_elements,
                    opts.alpha,
                    opts.arena_dir,
                )?))
            }
            crate::StorageMode::SQ8 => {
                HnswBackend::Sq8(Box::new(Sq8PrecisionHnsw::with_optional_arena_dir(
                    distance,
                    opts.dimension,
                    opts.max_connections,
                    opts.ef_construction,
                    opts.max_elements,
                    opts.alpha,
                    opts.arena_dir,
                )?))
            }
            _ => Self::new_standard_backend(
                distance,
                opts.max_connections,
                opts.ef_construction,
                opts.max_elements,
                opts.dimension,
                opts.alpha,
                opts.arena_dir,
            )?,
        };

        Ok(Self {
            backend,
            metric: opts.metric,
        })
    }

    /// Builds the Standard (full-f32) backend.
    ///
    /// A zero `dimension` means the caller does not know it yet, so the
    /// graph allocates its vector storage lazily on first insert instead of
    /// pre-sizing it.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    #[allow(clippy::too_many_arguments)]
    fn new_standard_backend(
        distance: CachedSimdDistance,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
        dimension: usize,
        alpha: f32,
        arena_dir: Option<&std::path::Path>,
    ) -> crate::error::Result<HnswBackend> {
        #[cfg(feature = "persistence")]
        if let Some(dir) = arena_dir {
            let inner = NativeHnsw::standard_in_dir(
                distance,
                max_connections,
                ef_construction,
                max_elements,
                dimension,
                alpha,
                dir,
            )?;
            return Ok(HnswBackend::Standard(inner));
        }
        #[cfg(not(feature = "persistence"))]
        let _ = arena_dir;
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
        Ok(HnswBackend::Standard(inner))
    }

    /// Returns the storage mode for this backend.
    #[must_use]
    pub fn storage_mode(&self) -> crate::StorageMode {
        match &self.backend {
            HnswBackend::Standard(_) => crate::StorageMode::Full,
            HnswBackend::RaBitQ(_) => crate::StorageMode::RaBitQ,
            HnswBackend::Sq8(_) => crate::StorageMode::SQ8,
        }
    }

    /// Returns the VAMANA alpha used by this backend's graph.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub(crate) fn alpha(&self) -> f32 {
        match &self.backend {
            HnswBackend::Standard(g) => g.get_alpha(),
            HnswBackend::RaBitQ(p) => p.inner.get_alpha(),
            HnswBackend::Sq8(p) => p.inner.get_alpha(),
        }
    }

    /// Installs a pre-trained `RaBitQ` quantizer into the `RaBitQ` backend,
    /// re-encoding every stored vector in `NodeId` order.
    ///
    /// Returns `Ok(true)` when installed, `Ok(false)` when the backend is
    /// Standard (no-op — the wiring takes effect at the next open, once the
    /// backend is rebuilt as `RaBitQ` from the collection storage mode).
    ///
    /// # Errors
    ///
    /// Returns an error if re-encoding a stored vector fails.
    #[cfg(feature = "persistence")]
    pub fn install_trained_rabitq(
        &self,
        rabitq: std::sync::Arc<crate::quantization::RaBitQIndex>,
    ) -> crate::error::Result<bool> {
        match &self.backend {
            HnswBackend::RaBitQ(precision) => {
                precision.install_trained_rabitq(rabitq)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Installs a pre-trained SQ8 quantizer into the SQ8 backend,
    /// re-encoding every stored vector in `NodeId` order.
    ///
    /// Returns `Ok(true)` when installed, `Ok(false)` when the backend is
    /// not SQ8 (no-op — the wiring takes effect at the next open, once the
    /// backend is rebuilt from the collection storage mode) or the codec
    /// does not support the metric (search stays exact f32).
    ///
    /// # Errors
    ///
    /// Returns an error if re-encoding a stored vector fails.
    #[cfg(feature = "persistence")]
    pub fn install_trained_sq8(
        &self,
        quantizer: std::sync::Arc<crate::index::hnsw::native::ScalarQuantizer>,
    ) -> crate::error::Result<bool> {
        match &self.backend {
            HnswBackend::Sq8(precision) => precision.install_trained_sq8(quantizer),
            _ => Ok(false),
        }
    }

    /// Returns `true` when this index runs a quantized backend (`RaBitQ` or
    /// SQ8) whose positional code store requires inserts to flow through the
    /// backend itself (no direct-writer / async-builder bypass).
    #[must_use]
    pub fn is_quantized_backend(&self) -> bool {
        matches!(self.backend, HnswBackend::RaBitQ(_) | HnswBackend::Sq8(_))
    }

    /// Converts a Standard backend into an (untrained) `RaBitQ` backend.
    ///
    /// Vacuum rebuilds insert through a Standard graph so lazy quantizer
    /// training can never fire mid-rebuild (it would train a throwaway
    /// quantizer from compaction order); the caller promotes afterwards and
    /// installs the carried-over quantizer, if any. A backend that is
    /// already `RaBitQ` is returned unchanged.
    pub fn promote_to_rabitq(self, dimension: usize) -> Self {
        match self.backend {
            HnswBackend::Standard(inner) => Self {
                backend: HnswBackend::RaBitQ(Box::new(RaBitQPrecisionHnsw::from_inner(
                    inner, dimension,
                ))),
                metric: self.metric,
            },
            backend => Self {
                backend,
                metric: self.metric,
            },
        }
    }

    /// Converts a Standard backend into an (untrained) SQ8 backend — the
    /// vacuum promotion mirror of [`Self::promote_to_rabitq`].
    pub fn promote_to_sq8(self, dimension: usize) -> Self {
        match self.backend {
            HnswBackend::Standard(inner) => Self {
                backend: HnswBackend::Sq8(Box::new(Sq8PrecisionHnsw::from_inner(inner, dimension))),
                metric: self.metric,
            },
            backend => Self {
                backend,
                metric: self.metric,
            },
        }
    }

    pub fn is_rabitq_quantizer_trained(&self) -> bool {
        matches!(&self.backend, HnswBackend::RaBitQ(precision) if precision.is_quantizer_trained())
    }

    /// Returns true when the backend is SQ8 with a trained quantizer.
    pub fn is_sq8_quantizer_trained(&self) -> bool {
        matches!(&self.backend, HnswBackend::Sq8(precision) if precision.is_quantizer_trained())
    }

    /// Returns the trained `RaBitQ` quantizer, if any.
    ///
    /// Used by vacuum to carry the trained rotation over to the rebuilt
    /// backend via [`Self::install_trained_rabitq`].
    #[cfg(feature = "persistence")]
    #[must_use]
    pub fn rabitq_quantizer(&self) -> Option<std::sync::Arc<crate::quantization::RaBitQIndex>> {
        match &self.backend {
            HnswBackend::RaBitQ(precision) => precision.trained_quantizer(),
            _ => None,
        }
    }

    /// Returns the trained SQ8 quantizer, if any.
    ///
    /// Used by vacuum and the collection flush path (persisting `sq8.idx`)
    /// to carry a trained quantizer over via [`Self::install_trained_sq8`].
    #[cfg(feature = "persistence")]
    #[must_use]
    pub fn sq8_quantizer(
        &self,
    ) -> Option<std::sync::Arc<crate::index::hnsw::native::ScalarQuantizer>> {
        match &self.backend {
            HnswBackend::Sq8(precision) => precision.trained_quantizer(),
            _ => None,
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
            HnswBackend::Sq8(sq8) => sq8.search(query, k, ef_search),
        }
    }

    /// Searches the HNSW graph, automatically choosing GPU or CPU path.
    ///
    /// When the GPU feature is enabled and the index exceeds the traversal
    /// threshold (500K vectors), attempts GPU-accelerated layer-0 search.
    /// Falls back to CPU on any GPU error or if GPU is unavailable.
    ///
    /// Returns raw distances in the same format as [`search`](Self::search) —
    /// the caller **must** call [`transform_score`](Self::transform_score)
    /// regardless of which path was taken. GPU shaders output HNSW-compatible
    /// distances (1-cosine, squared L2, -dot) matching CPU semantics.
    ///
    /// For `RaBitQ` backend, always uses CPU (binary distance GPU shader
    /// is not yet implemented).
    #[must_use]
    pub fn search_auto(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(usize, f32)> {
        if let Some(results) = self.try_gpu_route(query, k, ef_search) {
            return results;
        }
        self.search(query, k, ef_search)
    }

    /// GPU routing policy shared by [`Self::search_auto`] and
    /// [`Self::search_resumable`]: attempts device layer-0 traversal when the
    /// Standard backend crosses the GPU threshold. `None` means "take the CPU
    /// path" — either the feature is off, the backend/scale does not qualify,
    /// or the GPU attempt failed and search falls through.
    ///
    /// `query.len()` is authoritative for the index dimension: a query of
    /// wrong length would fail distance evaluation anyway.
    #[allow(unused_variables)] // Reason: parameters unused when `gpu` is off
    fn try_gpu_route(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Option<Vec<(usize, f32)>> {
        #[cfg(feature = "gpu")]
        if let HnswBackend::Standard(hnsw) = &self.backend {
            if crate::gpu::should_traverse_gpu(hnsw.len(), query.len()) {
                if let Some(results) = self.search_gpu(query, k, ef_search) {
                    return Some(results);
                }
                // GPU failed — fall through to CPU
            }
        }
        None
    }

    /// Phase-1 search that keeps its traversal state for a possible escalation.
    ///
    /// Same GPU/backend dispatch as [`Self::search_auto`], with one addition:
    /// on the Standard-backend CPU path the graph traversal state is returned
    /// as a [`SearchResume`] handle, so an escalating caller can widen the
    /// `ef` budget via [`Self::resume_search`] instead of restarting.
    ///
    /// Paths with no CPU-side state to resume return `None` and keep their
    /// pre-change escalation behavior (restart):
    /// - GPU layer-0 traversal (state lives on the device);
    /// - the `RaBitQ` backend (its binary-quantized search loop owns no
    ///   [`ResumableSearch`]).
    #[must_use]
    pub fn search_resumable(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> (Vec<(usize, f32)>, Option<SearchResume>) {
        if let Some(results) = self.try_gpu_route(query, k, ef_search) {
            // GPU traversal keeps no CPU-side state to resume.
            return (results, None);
        }

        match &self.backend {
            HnswBackend::Standard(hnsw) => {
                let (results, resume) = hnsw.search_resumable(query, k, ef_search);
                (results, resume.map(SearchResume))
            }
            HnswBackend::RaBitQ(rabitq) => (rabitq.search(query, k, ef_search), None),
            HnswBackend::Sq8(sq8) => (sq8.search(query, k, ef_search), None),
        }
    }

    /// Continues a [`Self::search_resumable`] traversal under a widened `ef`.
    ///
    /// `query` must be the same vector passed to `search_resumable`.
    #[must_use]
    pub fn resume_search(
        &self,
        resume: SearchResume,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<(usize, f32)> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.resume_search(resume.0, query, k, ef_search),
            // Unreachable in practice: `SearchResume` is only handed out for
            // the Standard backend. Kept total so a future backend cannot
            // silently drop the escalation.
            HnswBackend::RaBitQ(rabitq) => rabitq.search(query, k, ef_search),
            HnswBackend::Sq8(sq8) => sq8.search(query, k, ef_search),
        }
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

    /// Searches the HNSW graph and returns results as `NativeNeighbour` structs.
    #[allow(dead_code)] // Reason: API surface — used by callers needing typed neighbour results
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
            HnswBackend::RaBitQ(_) | HnswBackend::Sq8(_) => self
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
            HnswBackend::Sq8(sq8) => sq8.insert(vector)?,
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
            // Quantized backends: insert sequentially so the positional code
            // store stays consistent with NodeId assignment order.
            HnswBackend::RaBitQ(_) | HnswBackend::Sq8(_) => {
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
            HnswBackend::Sq8(sq8) => sq8.inner.set_searching_mode(mode),
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
            HnswBackend::Sq8(sq8) => sq8.inner.reorder_for_locality(),
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
            HnswBackend::Sq8(sq8) => sq8.inner.file_dump(path, basename),
        }
    }

    /// Loads the HNSW graph with a specific storage mode.
    ///
    /// A quantized `storage_mode` ([`crate::StorageMode::RaBitQ`] or
    /// [`crate::StorageMode::SQ8`]) wraps the loaded graph in the matching
    /// backend. The quantizer is NOT trained here — callers restore a
    /// persisted artifact via [`Self::install_trained_rabitq`] /
    /// [`Self::install_trained_sq8`] or let it train lazily from inserts.
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
        let distance = CachedSimdDistance::new_prenormalized(metric, dimension);
        // Same gate as `build`, for the same reason: only a backend that
        // traverses on codes can afford an evictable f32 arena. `path` is the
        // collection directory, which is where the arena file belongs — it is
        // a cache of `{basename}.vectors`, deleted when this graph drops.
        let arena_dir = match storage_mode {
            crate::StorageMode::RaBitQ | crate::StorageMode::SQ8 => Some(path),
            _ => None,
        };
        let inner = NativeHnsw::file_load_with_arena(path, basename, distance, arena_dir)?;

        let backend = match storage_mode {
            // Wrap the loaded graph in the matching quantized backend. The
            // quantizer is NOT trained yet — callers install a persisted
            // artifact or let it train lazily from inserts.
            crate::StorageMode::RaBitQ => {
                HnswBackend::RaBitQ(Box::new(RaBitQPrecisionHnsw::from_inner(inner, dimension)))
            }
            crate::StorageMode::SQ8 => {
                HnswBackend::Sq8(Box::new(Sq8PrecisionHnsw::from_inner(inner, dimension)))
            }
            _ => HnswBackend::Standard(inner),
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
            HnswBackend::RaBitQ(_) | HnswBackend::Sq8(_) => raw_distance,
        }
    }

    /// Returns the number of elements in the index.
    #[allow(dead_code)] // Reason: API surface — introspection accessor for callers
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.len(),
            HnswBackend::RaBitQ(rabitq) => rabitq.len(),
            HnswBackend::Sq8(sq8) => sq8.len(),
        }
    }

    /// Returns true if the index is empty.
    #[allow(dead_code)] // Reason: API surface — emptiness check paired with `len()`
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.is_empty(),
            HnswBackend::RaBitQ(rabitq) => rabitq.is_empty(),
            HnswBackend::Sq8(sq8) => sq8.is_empty(),
        }
    }

    /// Returns the distance metric used by this index.
    #[allow(dead_code)] // Reason: API surface — metric accessor for callers
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
    #[allow(dead_code)] // Reason: test-only callers (direct_writer_tests) — kept for API symmetry with _mut
    #[inline]
    pub fn with_contiguous_vectors_read<R: Default>(
        &self,
        f: impl FnOnce(&crate::perf_optimizations::ContiguousVectors) -> R,
    ) -> R {
        self.with_contiguous_vectors(f)
    }

    /// Executes a closure with mutable access to the contiguous vector storage.
    ///
    /// Acquires a write lock on the underlying `NativeHnsw.vectors` `RwLock`.
    /// Used by `DirectVectorWriter` to write vectors directly during bulk insert.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Internal`] if vector storage is not initialized.
    /// Propagates any error returned by the closure.
    ///
    /// [`crate::error::Error::Internal`]: crate::error::Error::Internal
    pub fn with_contiguous_vectors_mut<R>(
        &self,
        f: impl FnOnce(&mut crate::perf_optimizations::ContiguousVectors) -> crate::error::Result<R>,
    ) -> crate::error::Result<R> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.with_vectors_write(f),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.with_vectors_write(f),
            HnswBackend::Sq8(sq8) => sq8.inner.with_vectors_write(f),
        }
    }
}

// ============================================================================
// Send + Sync for thread safety
// ============================================================================

// `NativeHnswInner` is auto-`Send + Sync`: every field is either lock/atomic
// protected or itself `Send + Sync`. Its only raw pointer lives in
// `ContiguousVectors` (`perf_optimizations.rs`), which carries its own audited
// `unsafe impl Send`/`Sync` — so that `NonNull` is already "whitewashed" one
// level down, and `NativeHnswInner` needs no `unsafe impl` of its own. An
// unconditional `unsafe impl` here would be worse than nothing: it would
// silently mask a future non-`Send` field. The compile-time assertion below is
// the real guard — with no `unsafe impl` shadowing it, adding a `!Send`/`!Sync`
// field breaks the build here instead of introducing a subtle data race.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<NativeHnswInner>();
};

// ============================================================================
// Tests moved to native_inner_tests.rs per project rules
