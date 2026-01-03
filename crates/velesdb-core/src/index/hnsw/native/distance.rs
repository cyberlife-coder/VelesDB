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

/// SIMD-accelerated distance computation.
///
/// Uses AVX2/AVX-512 on x86_64, NEON on ARM.
pub struct SimdDistance {
    metric: DistanceMetric,
}

impl SimdDistance {
    #[must_use]
    pub fn new(metric: DistanceMetric) -> Self {
        Self { metric }
    }
}

impl DistanceEngine for SimdDistance {
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        // Use our existing optimized SIMD functions
        match self.metric {
            DistanceMetric::Cosine => 1.0 - crate::simd::cosine_similarity_fast(a, b),
            DistanceMetric::Euclidean => crate::simd::euclidean_distance_fast(a, b),
            DistanceMetric::DotProduct => -crate::simd::dot_product_fast(a, b), // Negate for distance
            // Hamming/Jaccard use scalar for now (no SIMD impl)
            DistanceMetric::Hamming => hamming_distance_scalar(a, b),
            DistanceMetric::Jaccard => jaccard_distance_scalar(a, b),
        }
    }

    fn batch_distance(&self, query: &[f32], candidates: &[&[f32]]) -> Vec<f32> {
        // TODO: Optimize with prefetching and parallel processing
        candidates.iter().map(|c| self.distance(query, c)).collect()
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
    a.iter()
        .zip(b.iter())
        .filter(|(x, y)| (x.to_bits() ^ y.to_bits()) != 0)
        .count() as f32
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

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_identical_vectors() {
        let engine = CpuDistance::new(DistanceMetric::Cosine);
        let v = vec![1.0, 2.0, 3.0];
        let dist = engine.distance(&v, &v);
        assert!(
            dist.abs() < 1e-5,
            "Identical vectors should have distance ~0"
        );
    }

    #[test]
    fn test_euclidean_known_distance() {
        let engine = CpuDistance::new(DistanceMetric::Euclidean);
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        let dist = engine.distance(&a, &b);
        assert!((dist - 5.0).abs() < 1e-5, "3-4-5 triangle");
    }

    #[test]
    fn test_simd_matches_scalar() {
        let cpu = CpuDistance::new(DistanceMetric::Cosine);
        let simd = SimdDistance::new(DistanceMetric::Cosine);

        let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
        let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

        let cpu_dist = cpu.distance(&a, &b);
        let simd_dist = simd.distance(&a, &b);

        assert!(
            (cpu_dist - simd_dist).abs() < 1e-4,
            "SIMD should match scalar: cpu={cpu_dist}, simd={simd_dist}"
        );
    }
}
