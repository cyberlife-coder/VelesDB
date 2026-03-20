//! HNSW Graph Structure
//!
//! Implements the hierarchical navigable small world graph structure
//! as described in the Malkov & Yashunin paper.
//!
//! # Module Organization
//!
//! - `insert`: Vector insertion and layer growth
//! - `search`: k-NN search, multi-entry search, and layer-level search
//! - `neighbors`: Neighbor selection (VAMANA diversification) and bidirectional connections

mod insert;
pub(crate) mod locking;
mod neighbors;
mod reorder;
pub(crate) mod safety_counters;
mod search;

use super::distance::DistanceEngine;
use super::layer::{Layer, NodeId};
use crate::perf_optimizations::ContiguousVectors;
use locking::{record_lock_acquire, record_lock_release, LockRank};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Native HNSW index implementation.
///
/// # Type Parameters
///
/// * `D` - Distance engine (CPU, SIMD, or GPU)
pub struct NativeHnsw<D: DistanceEngine> {
    /// Distance computation engine
    pub(in crate::index::hnsw::native) distance: D,
    /// Contiguous vector storage (node_id -> vector slice).
    /// `None` until the first vector is inserted (dimension is inferred lazily).
    pub(in crate::index::hnsw::native) vectors: RwLock<Option<ContiguousVectors>>,
    /// Hierarchical layers (layer 0 = bottom, dense connections)
    pub(in crate::index::hnsw::native) layers: RwLock<Vec<Layer>>,
    /// Entry point for search (highest layer node)
    pub(in crate::index::hnsw::native) entry_point: RwLock<Option<NodeId>>,
    /// Maximum layer for entry point
    pub(in crate::index::hnsw::native) max_layer: AtomicUsize,
    /// Number of elements in the index
    pub(in crate::index::hnsw::native) count: AtomicUsize,
    /// Simple PRNG state for layer selection
    pub(in crate::index::hnsw::native) rng_state: AtomicU64,
    /// Maximum connections per node (M parameter)
    pub(in crate::index::hnsw::native) max_connections: usize,
    /// Maximum connections at layer 0 (M0 = 2*M)
    pub(in crate::index::hnsw::native) max_connections_0: usize,
    /// ef_construction parameter
    pub(in crate::index::hnsw::native) ef_construction: usize,
    /// Level multiplier for layer selection (1/ln(M))
    pub(in crate::index::hnsw::native) level_mult: f64,
    /// VAMANA alpha parameter for neighbor diversification (default: 1.0)
    pub(in crate::index::hnsw::native) alpha: f32,
    /// Maximum consecutive candidates without improving top-k before early termination.
    /// Default: `ef_construction / 4`. Set to `0` to disable.
    pub(crate) stagnation_limit: usize,
}

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Creates a new native HNSW index.
    ///
    /// Vector storage is initialized lazily on the first `insert()` call,
    /// using the dimension of the first inserted vector.
    #[must_use]
    pub fn new(
        distance: D,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
    ) -> Self {
        Self::build(
            distance,
            max_connections,
            ef_construction,
            max_elements,
            1.0,
            None,
        )
    }

    /// Creates a new native HNSW index with a known vector dimension.
    ///
    /// Pre-allocates contiguous vector storage for cache-friendly access.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector storage allocation fails.
    pub fn new_with_dimension(
        distance: D,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
        dimension: usize,
    ) -> crate::error::Result<Self> {
        let storage = ContiguousVectors::new(dimension, max_elements)?;
        Ok(Self::build(
            distance,
            max_connections,
            ef_construction,
            max_elements,
            1.0,
            Some(storage),
        ))
    }

    /// Creates a new native HNSW index with VAMANA-style diversification.
    #[must_use]
    pub fn with_alpha(
        distance: D,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
        alpha: f32,
    ) -> Self {
        Self::build(
            distance,
            max_connections,
            ef_construction,
            max_elements,
            alpha,
            None,
        )
    }

    /// Internal constructor shared by all public constructors.
    fn build(
        distance: D,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
        alpha: f32,
        vectors: Option<ContiguousVectors>,
    ) -> Self {
        let max_connections_0 = max_connections * 2;
        let level_mult = 1.0 / (max_connections as f64).ln();
        Self {
            distance,
            vectors: RwLock::new(vectors),
            layers: RwLock::new(vec![Layer::new(max_elements)]),
            entry_point: RwLock::new(None),
            max_layer: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
            rng_state: AtomicU64::new(0x5DEE_CE66_D1A4_B5B5),
            max_connections,
            max_connections_0,
            ef_construction,
            level_mult,
            alpha,
            stagnation_limit: ef_construction / 4,
        }
    }

    /// Returns the alpha diversification parameter.
    #[must_use]
    pub fn get_alpha(&self) -> f32 {
        self.alpha
    }

    /// Returns the number of elements in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// Returns true if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Computes the distance between two vectors using this index's distance engine.
    #[inline]
    #[must_use]
    pub fn compute_distance(&self, a: &[f32], b: &[f32]) -> f32 {
        self.distance.distance(a, b)
    }

    /// Executes a closure with a vectors read snapshot and tracked lock rank.
    ///
    /// The closure receives `&ContiguousVectors`; if storage is not yet
    /// initialized (no vectors inserted), the closure is **not** called and
    /// the supplied `default` value is returned instead.
    #[inline]
    pub(in crate::index::hnsw) fn with_vectors_read<R>(
        &self,
        f: impl FnOnce(&ContiguousVectors) -> R,
    ) -> R
    where
        R: Default,
    {
        record_lock_acquire(LockRank::Vectors);
        let guard = self.vectors.read();
        let result = match guard.as_ref() {
            Some(v) => f(v),
            None => R::default(),
        };
        drop(guard);
        record_lock_release(LockRank::Vectors);
        result
    }

    /// Executes a closure with a layers read snapshot and tracked lock rank.
    #[inline]
    pub(in crate::index::hnsw::native) fn with_layers_read<R>(
        &self,
        f: impl FnOnce(&[Layer]) -> R,
    ) -> R {
        record_lock_acquire(LockRank::Layers);
        let layers = self.layers.read();
        let result = f(&layers);
        drop(layers);
        record_lock_release(LockRank::Layers);
        result
    }

    /// Executes a closure with both vectors AND layers read locks held simultaneously.
    ///
    /// Acquires locks in correct rank order: vectors (10) → layers (20).
    /// This avoids repeated lock acquire/release in tight search loops (F-03/F-04).
    ///
    /// If vector storage is not yet initialized, the closure is **not** called
    /// and `R::default()` is returned.
    #[inline]
    pub(in crate::index::hnsw::native) fn with_vectors_and_layers_read<R>(
        &self,
        f: impl FnOnce(&ContiguousVectors, &[Layer]) -> R,
    ) -> R
    where
        R: Default,
    {
        record_lock_acquire(LockRank::Vectors);
        let vectors_guard = self.vectors.read();
        let Some(vectors) = vectors_guard.as_ref() else {
            drop(vectors_guard);
            record_lock_release(LockRank::Vectors);
            return R::default();
        };
        record_lock_acquire(LockRank::Layers);
        let layers = self.layers.read();
        let result = f(vectors, &layers);
        drop(layers);
        record_lock_release(LockRank::Layers);
        drop(vectors_guard);
        record_lock_release(LockRank::Vectors);
        result
    }

    // SAFETY: Layer selection uses exponential distribution capped at 15.
    // - cast_precision_loss: u64 to f64 may lose precision but is acceptable for PRNG
    // - cast_possible_truncation: floor() result is capped at 15, fitting in usize
    // - cast_sign_loss: -ln(uniform) is always positive since uniform is in (0, 1)
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn random_layer(&self) -> usize {
        let old_state = self
            .rng_state
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |mut state| {
                if state == 0 {
                    state = 0x853c_49e6_748f_ea9b;
                }
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                Some(state)
            })
            .unwrap_or_else(|s| s);
        let mut state = old_state;
        if state == 0 {
            state = 0x853c_49e6_748f_ea9b;
        }
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let uniform = (state as f64) / (u64::MAX as f64);
        let uniform_safe = uniform.max(f64::MIN_POSITIVE);
        let level = (-uniform_safe.ln() * self.level_mult).floor() as usize;
        level.min(15)
    }
}
