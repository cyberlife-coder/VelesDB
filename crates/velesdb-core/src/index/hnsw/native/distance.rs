//! Distance computation engines for native HNSW.
//!
//! Provides trait abstraction for different distance computation backends:
//! - CPU scalar (baseline)
//! - CPU SIMD (AVX2/AVX-512/NEON)
//! - GPU (future: CUDA/Vulkan compute)

use crate::distance::DistanceMetric;

/// Trait for distance computation engines.
///
/// This abstraction allows swapping between CPU, SIMD, and GPU backends
/// without changing the HNSW algorithm implementation.
pub trait DistanceEngine: Send + Sync {
    /// Computes distance between two vectors.
    fn distance(&self, a: &[f32], b: &[f32]) -> f32;

    /// Batch distance computation (one query vs many candidates).
    ///
    /// Returns distances in the same order as candidates.
    /// Default implementation calls `distance()` in a loop.
    fn batch_distance(&self, query: &[f32], candidates: &[&[f32]]) -> Vec<f32> {
        candidates.iter().map(|c| self.distance(query, c)).collect()
    }

    /// Returns the metric type for this engine.
    fn metric(&self) -> DistanceMetric;
}

/// CPU scalar distance computation (baseline, no SIMD).
pub struct CpuDistance {
    metric: DistanceMetric,
}

impl CpuDistance {
    /// Creates a new CPU distance engine with the given metric.
    #[must_use]
    pub fn new(metric: DistanceMetric) -> Self {
        Self { metric }
    }
}

impl DistanceEngine for CpuDistance {
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.metric {
            DistanceMetric::Cosine => cosine_distance_scalar(a, b),
            DistanceMetric::Euclidean => euclidean_distance_scalar(a, b),
            DistanceMetric::DotProduct => dot_product_scalar(a, b),
            DistanceMetric::Hamming => hamming_distance_scalar(a, b),
            DistanceMetric::Jaccard => jaccard_distance_scalar(a, b),
        }
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

/// SIMD-accelerated distance computation with prefetch optimization.
///
/// Delegates all metric calculations to `simd_native` module which handles
/// AVX-512/AVX2/NEON dispatch. The `batch_distance` implementation adds
/// CPU prefetch hints to hide memory latency during batch operations.
///
/// Note: `NativeSimdDistance` and `AdaptiveSimdDistance` share the same
/// `distance()` implementation. They differ only in `batch_distance` strategy.
pub struct SimdDistance {
    metric: DistanceMetric,
}

impl SimdDistance {
    /// Creates a new SIMD-accelerated distance engine with the given metric.
    #[must_use]
    pub fn new(metric: DistanceMetric) -> Self {
        Self { metric }
    }
}

impl DistanceEngine for SimdDistance {
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        // Use our existing optimized SIMD functions for ALL metrics
        match self.metric {
            DistanceMetric::Cosine => 1.0 - crate::simd_native::cosine_similarity_native(a, b),
            DistanceMetric::Euclidean => crate::simd_native::euclidean_native(a, b),
            DistanceMetric::DotProduct => -crate::simd_native::dot_product_native(a, b),
            DistanceMetric::Hamming => crate::simd_native::hamming_distance_native(a, b),
            DistanceMetric::Jaccard => 1.0 - crate::simd_native::jaccard_similarity_native(a, b),
        }
    }

    fn batch_distance(&self, query: &[f32], candidates: &[&[f32]]) -> Vec<f32> {
        // PERF-2: Optimized batch distance with CPU prefetch hints
        // Prefetch upcoming vectors to hide memory latency
        let prefetch_distance = crate::simd_native::calculate_prefetch_distance(query.len());
        let mut results = Vec::with_capacity(candidates.len());

        for (i, candidate) in candidates.iter().enumerate() {
            // Prefetch upcoming candidate vectors into L1 cache
            if i + prefetch_distance < candidates.len() {
                crate::simd_native::prefetch_vector(candidates[i + prefetch_distance]);
            }
            results.push(self.distance(query, candidate));
        }

        results
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

/// Native SIMD distance computation using simd_native dispatch.
///
/// Delegates to `simd_native` module which handles AVX-512/AVX2/NEON dispatch
/// based on CPU capabilities and vector size.
///
/// Note: `distance()` is identical to `SimdDistance`. The difference is in
/// `batch_distance`: this engine uses `batch_dot_product_native` for
/// `DotProduct` metric (vectorized batch) and falls back to per-item
/// distance for other metrics (no prefetch hints).
pub struct NativeSimdDistance {
    metric: DistanceMetric,
}

impl NativeSimdDistance {
    /// Creates a new native SIMD distance engine.
    #[must_use]
    pub fn new(metric: DistanceMetric) -> Self {
        Self { metric }
    }
}

impl DistanceEngine for NativeSimdDistance {
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.metric {
            DistanceMetric::Cosine => 1.0 - crate::simd_native::cosine_similarity_native(a, b),
            DistanceMetric::Euclidean => crate::simd_native::euclidean_native(a, b),
            DistanceMetric::DotProduct => -crate::simd_native::dot_product_native(a, b),
            // Use simd_native directly for Hamming/Jaccard (EPIC-081 consolidation)
            DistanceMetric::Hamming => crate::simd_native::hamming_distance_native(a, b),
            DistanceMetric::Jaccard => 1.0 - crate::simd_native::jaccard_similarity_native(a, b),
        }
    }

    fn batch_distance(&self, query: &[f32], candidates: &[&[f32]]) -> Vec<f32> {
        match self.metric {
            DistanceMetric::DotProduct => {
                // Use optimized batch with prefetch
                crate::simd_native::batch_dot_product_native(candidates, query)
                    .into_iter()
                    .map(|d| -d)
                    .collect()
            }
            _ => candidates.iter().map(|c| self.distance(query, c)).collect(),
        }
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

/// SIMD distance computation with prefetch-based batching.
///
/// Note: This engine is functionally identical to `SimdDistance` — both
/// `distance()` and `batch_distance()` produce the same results via the
/// same code paths. Retained for backward compatibility.
pub struct AdaptiveSimdDistance {
    metric: DistanceMetric,
}

impl AdaptiveSimdDistance {
    /// Creates a new adaptive SIMD distance engine.
    #[must_use]
    pub fn new(metric: DistanceMetric) -> Self {
        Self { metric }
    }
}

impl DistanceEngine for AdaptiveSimdDistance {
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        // Calculate distance directly using simd_native (consolidated implementation)
        match self.metric {
            DistanceMetric::Cosine => 1.0 - crate::simd_native::cosine_similarity_native(a, b),
            DistanceMetric::Euclidean => crate::simd_native::euclidean_native(a, b),
            DistanceMetric::DotProduct => -crate::simd_native::dot_product_native(a, b),
            DistanceMetric::Hamming => crate::simd_native::hamming_distance_native(a, b),
            DistanceMetric::Jaccard => 1.0 - crate::simd_native::jaccard_similarity_native(a, b),
        }
    }

    fn batch_distance(&self, query: &[f32], candidates: &[&[f32]]) -> Vec<f32> {
        // Use prefetch optimization for batch operations
        let prefetch_distance = crate::simd_native::calculate_prefetch_distance(query.len());
        let mut results = Vec::with_capacity(candidates.len());

        for (i, candidate) in candidates.iter().enumerate() {
            // Prefetch upcoming candidate vectors into L1 cache
            if i + prefetch_distance < candidates.len() {
                crate::simd_native::prefetch_vector(candidates[i + prefetch_distance]);
            }
            results.push(self.distance(query, candidate));
        }

        results
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

// =============================================================================
// Scalar implementations (baseline for comparison)
// =============================================================================

#[inline]
fn cosine_distance_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom == 0.0 {
        1.0
    } else {
        1.0 - (dot / denom)
    }
}

#[inline]
fn euclidean_distance_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

#[inline]
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    // Return negative because we want distance (lower = better)
    -a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>()
}

#[inline]
fn hamming_distance_scalar(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY (cast): Vector dimensions are bounded by collection config (max 65536).
    // f32 has 24-bit mantissa, so counts up to 2^24 (16M) are exact.
    #[allow(clippy::cast_precision_loss)]
    let count = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| (x.to_bits() ^ y.to_bits()) != 0)
        .count() as f32;
    count
}

#[inline]
fn jaccard_distance_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut intersection = 0.0_f32;
    let mut union = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        intersection += x.min(*y);
        union += x.max(*y);
    }

    if union == 0.0 {
        1.0
    } else {
        1.0 - (intersection / union)
    }
}
