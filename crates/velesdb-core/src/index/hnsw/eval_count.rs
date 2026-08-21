//! Bench-only counter of single-pair distance evaluations.
//!
//! Compiled only with the `internal-bench` feature: release and default test
//! builds carry zero instrumentation. The counter is a process-global relaxed
//! atomic incremented at the two single-pair distance entry points the query
//! paths funnel through:
//!
//! - [`DistanceEngine::distance`](super::native::distance) impls
//!   (`CachedSimdDistance`, `CpuDistance`) — HNSW graph traversal and the
//!   batch/prefetch helper built on it;
//! - `HnswIndex::compute_distance` — the brute-force bitmap scan and SIMD
//!   reranking paths, which call `simd_native` kernels directly.
//!
//! The count is a *work measure*, not a timer: it is bit-for-bit reproducible
//! across runs and machines for a deterministic corpus (the HNSW level PRNG
//! is fixed-seed), which makes it usable on shared CI runners where
//! wall-clock benchmarks are noise. Block-columnar (PDX) kernels compute
//! distances in blocks without passing through these entry points, so for
//! paths that engage them the counter is a lower bound.
//!
//! Corollary: with `internal-bench` enabled, the relaxed atomic increment
//! sits inside the distance hot loop — wall-clock measured under this
//! feature is NOT a benchmark of the real engine. Count with this feature;
//! time without it.

use std::sync::atomic::{AtomicU64, Ordering};

/// Total single-pair distance evaluations since the last reset.
static DISTANCE_EVALS: AtomicU64 = AtomicU64::new(0);

/// Records one single-pair distance evaluation.
#[inline]
pub(crate) fn record_eval() {
    DISTANCE_EVALS.fetch_add(1, Ordering::Relaxed);
}

/// Returns the number of single-pair distance evaluations since the last reset.
#[must_use]
pub(crate) fn distance_evals() -> u64 {
    DISTANCE_EVALS.load(Ordering::Relaxed)
}

/// Resets the evaluation counter to zero.
pub(crate) fn reset_distance_evals() {
    DISTANCE_EVALS.store(0, Ordering::Relaxed);
}
