//! Distance computation engines for native HNSW.
//!
//! Provides trait abstraction for different distance computation backends:
//! - CPU scalar (baseline)
//! - CPU SIMD (AVX2/AVX-512/NEON)
//! - GPU (future: CUDA/Vulkan compute)

use crate::distance::DistanceMetric;
use smallvec::SmallVec;

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

    /// Returns whether the engine expects cosine inputs to be pre-normalized.
    #[must_use]
    fn is_pre_normalized(&self) -> bool {
        false
    }
}

// =============================================================================
// Shared SIMD distance helpers (RF-DEDUP: eliminates 3x copy-paste)
// =============================================================================

/// Computes SIMD-accelerated distance for any metric via `simd_native`.
///
/// This is the single source of truth for metric-to-SIMD dispatch.
/// All SIMD-based `DistanceEngine` implementations delegate here.
#[inline]
pub(crate) fn simd_distance_for_metric(metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        DistanceMetric::Cosine => 1.0 - crate::simd_native::cosine_similarity_native(a, b),
        DistanceMetric::Euclidean => crate::simd_native::euclidean_native(a, b),
        DistanceMetric::DotProduct => -crate::simd_native::dot_product_native(a, b),
        DistanceMetric::Hamming => crate::simd_native::hamming_distance_native(a, b),
        DistanceMetric::Jaccard => 1.0 - crate::simd_native::jaccard_similarity_native(a, b),
    }
}

/// Batch distance with CPU prefetch hints to hide memory latency.
///
/// Returns `SmallVec<[f32; 32]>` to avoid heap allocation for typical
/// batch sizes (M=16..32 neighbors). For batches up to 32 elements
/// (~128 bytes), the result lives entirely on the stack.
///
/// Used by `CachedSimdDistance` and called directly from the HNSW search
/// hot loop to bypass the
/// `DistanceEngine::batch_distance` trait method (which returns `Vec<f32>`).
#[inline]
pub(crate) fn batch_distance_with_prefetch(
    engine: &impl DistanceEngine,
    query: &[f32],
    candidates: &[&[f32]],
) -> SmallVec<[f32; 32]> {
    let prefetch_distance = crate::simd_native::calculate_prefetch_distance(query.len());
    let mut results = SmallVec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        if i + prefetch_distance < candidates.len() {
            crate::simd_native::prefetch_vector(candidates[i + prefetch_distance]);
        }
        results.push(engine.distance(query, candidate));
    }

    results
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

/// SIMD distance with fully cached kernel resolution for HNSW hot loops.
///
/// All 5 distance metrics use pre-resolved fn pointers via `simd_native::DistanceEngine`.
/// The only per-call branch is `match self.metric` for the distance-to-similarity
/// transform (1-cosine, -dot), which is perfectly predicted by the branch predictor
/// since a given HNSW index always uses the same metric.
pub struct CachedSimdDistance {
    metric: DistanceMetric,
    engine: crate::simd_native::DistanceEngine,
    pre_normalized: bool,
}

impl CachedSimdDistance {
    /// Creates a cached SIMD distance engine optimized for the given metric and dimension.
    #[must_use]
    pub fn new(metric: DistanceMetric, dimension: usize) -> Self {
        Self {
            metric,
            engine: crate::simd_native::DistanceEngine::new(dimension),
            pre_normalized: false,
        }
    }

    /// Creates a cached SIMD engine that expects cosine vectors to be pre-normalized.
    ///
    /// For non-cosine metrics, this is equivalent to [`Self::new`].
    #[must_use]
    pub fn new_prenormalized(metric: DistanceMetric, dimension: usize) -> Self {
        Self {
            metric,
            engine: crate::simd_native::DistanceEngine::new(dimension),
            pre_normalized: metric == DistanceMetric::Cosine,
        }
    }
}

impl DistanceEngine for CachedSimdDistance {
    #[allow(clippy::inline_always)] // Reason: HNSW hot loop --- single branch + fn pointer call
    #[inline(always)]
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.metric {
            DistanceMetric::Cosine if self.pre_normalized => {
                1.0 - self.engine.cosine_similarity(a, b).clamp(-1.0, 1.0)
            }
            DistanceMetric::Cosine => 1.0 - self.engine.cosine_similarity(a, b),
            // Reason: Returns squared L2 (no sqrt) because HNSW traversal only
            // needs ordering and sqrt is monotone. The sqrt is deferred to
            // `transform_score()` which is applied to the final k results only.
            // This saves one f32::sqrt() per distance computation in the hot loop.
            DistanceMetric::Euclidean => self.engine.euclidean_squared(a, b),
            DistanceMetric::DotProduct => -self.engine.dot_product(a, b),
            DistanceMetric::Hamming => self.engine.hamming(a, b),
            DistanceMetric::Jaccard => 1.0 - self.engine.jaccard(a, b),
        }
    }

    fn batch_distance(&self, query: &[f32], candidates: &[&[f32]]) -> Vec<f32> {
        batch_distance_with_prefetch(self, query, candidates).into_vec()
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }

    fn is_pre_normalized(&self) -> bool {
        self.pre_normalized
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
