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

/// SIMD-accelerated distance computation.
///
/// Uses AVX2/AVX-512 on x86_64, NEON on ARM.
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
/// Delegates to simd_native module which handles AVX-512/AVX2/NEON dispatch
/// based on CPU capabilities and vector size. Part of EPIC-081 consolidation.
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

/// Direct SIMD distance computation with zero-overhead dispatch.
///
/// Uses `simd_native` module which directly calls SIMD intrinsics
/// (AVX-512, AVX2, NEON) without intermediate dispatch overhead.
/// This is the recommended engine for production use.
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

    // =========================================================================
    // TDD Tests for PERF-2: Hamming/Jaccard SIMD + batch_distance optimization
    // =========================================================================

    #[test]
    fn test_simd_hamming_uses_simd_implementation() {
        let simd = SimdDistance::new(DistanceMetric::Hamming);

        // Binary-like vectors (0.0 or 1.0)
        let a: Vec<f32> = (0..64)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..64)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();

        let dist = simd.distance(&a, &b);

        // Verify result is reasonable (hamming distance between these patterns)
        assert!(dist >= 0.0, "Hamming distance must be non-negative");
        assert!(dist <= 64.0, "Hamming distance cannot exceed vector length");
    }

    #[test]
    fn test_simd_jaccard_uses_simd_implementation() {
        let simd = SimdDistance::new(DistanceMetric::Jaccard);

        // Binary-like vectors for set similarity
        let a: Vec<f32> = (0..64).map(|i| if i < 32 { 1.0 } else { 0.0 }).collect();
        let b: Vec<f32> = (0..64).map(|i| if i < 48 { 1.0 } else { 0.0 }).collect();

        let dist = simd.distance(&a, &b);

        // Jaccard distance = 1 - similarity, should be in [0, 1]
        assert!(
            (0.0..=1.0).contains(&dist),
            "Jaccard distance must be in [0,1]"
        );

        // Intersection = 32, Union = 48, Similarity = 32/48 = 0.667, Distance = 0.333
        let expected = 1.0 - (32.0 / 48.0);
        assert!(
            (dist - expected).abs() < 1e-4,
            "Jaccard distance: expected {expected}, got {dist}"
        );
    }

    #[test]
    fn test_simd_hamming_identical_vectors() {
        let simd = SimdDistance::new(DistanceMetric::Hamming);
        let v: Vec<f32> = (0..32)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();

        let dist = simd.distance(&v, &v);
        assert!(
            dist.abs() < 1e-5,
            "Identical vectors should have distance 0"
        );
    }

    #[test]
    fn test_simd_jaccard_identical_vectors() {
        let simd = SimdDistance::new(DistanceMetric::Jaccard);
        let v: Vec<f32> = (0..32)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();

        let dist = simd.distance(&v, &v);
        assert!(
            dist.abs() < 1e-5,
            "Identical vectors should have distance 0"
        );
    }

    #[test]
    fn test_batch_distance_with_prefetch() {
        let simd = SimdDistance::new(DistanceMetric::Cosine);

        let query: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
        let candidates: Vec<Vec<f32>> = (0..100)
            .map(|j| {
                (0..768)
                    .map(|i| ((i + j * 10) as f32 * 0.01).cos())
                    .collect()
            })
            .collect();

        let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

        let distances = simd.batch_distance(&query, &candidate_refs);

        assert_eq!(distances.len(), 100, "Should return 100 distances");

        // Verify all distances are valid (cosine distance in [0, 2])
        for (i, &d) in distances.iter().enumerate() {
            assert!((0.0..=2.0).contains(&d), "Distance {i} = {d} out of range");
        }
    }

    #[test]
    fn test_batch_distance_consistency() {
        let simd = SimdDistance::new(DistanceMetric::Euclidean);

        let query: Vec<f32> = (0..128).map(|i| i as f32).collect();
        let candidates: Vec<Vec<f32>> = (0..20)
            .map(|j| (0..128).map(|i| (i + j) as f32).collect())
            .collect();

        let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

        // Batch distance
        let batch_distances = simd.batch_distance(&query, &candidate_refs);

        // Individual distances
        let individual_distances: Vec<f32> = candidate_refs
            .iter()
            .map(|c| simd.distance(&query, c))
            .collect();

        // Results should match exactly
        for (i, (batch, individual)) in batch_distances
            .iter()
            .zip(individual_distances.iter())
            .enumerate()
        {
            assert!(
                (batch - individual).abs() < 1e-6,
                "Mismatch at {i}: batch={batch}, individual={individual}"
            );
        }
    }

    #[test]
    fn test_batch_distance_empty() {
        let simd = SimdDistance::new(DistanceMetric::Cosine);
        let query = vec![1.0, 2.0, 3.0];
        let candidates: Vec<&[f32]> = vec![];

        let distances = simd.batch_distance(&query, &candidates);
        assert!(distances.is_empty(), "Empty candidates should return empty");
    }

    // =========================================================================
    // Tests for NativeSimdDistance (AVX-512/NEON intrinsics)
    // =========================================================================

    #[test]
    fn test_native_simd_matches_simd() {
        let simd = SimdDistance::new(DistanceMetric::Cosine);
        let native = super::NativeSimdDistance::new(DistanceMetric::Cosine);

        let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
        let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

        let simd_dist = simd.distance(&a, &b);
        let native_dist = native.distance(&a, &b);

        assert!(
            (simd_dist - native_dist).abs() < 1e-3,
            "Native SIMD should match SIMD: simd={simd_dist}, native={native_dist}"
        );
    }

    #[test]
    fn test_native_simd_euclidean() {
        let native = super::NativeSimdDistance::new(DistanceMetric::Euclidean);

        let a = vec![0.0, 0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0, 0.0];

        let dist = native.distance(&a, &b);
        assert!((dist - 5.0).abs() < 1e-5, "3-4-5 triangle: got {dist}");
    }

    #[test]
    fn test_native_simd_dot_product() {
        let native = super::NativeSimdDistance::new(DistanceMetric::DotProduct);

        let a: Vec<f32> = (0..128).map(|i| i as f32 * 0.1).collect();
        let b: Vec<f32> = (0..128).map(|i| (128 - i) as f32 * 0.1).collect();

        let dist = native.distance(&a, &b);
        // DotProduct distance is negative dot product
        assert!(dist < 0.0, "DotProduct distance should be negative");
    }

    // =========================================================================
    // Additional tests for 90% coverage
    // =========================================================================

    #[test]
    fn test_cpu_distance_dot_product() {
        let cpu = CpuDistance::new(DistanceMetric::DotProduct);
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let dist = cpu.distance(&a, &b);
        // dot = 1*4 + 2*5 + 3*6 = 32, distance = -32
        assert!((dist + 32.0).abs() < 1e-5);
    }

    #[test]
    fn test_cpu_distance_hamming() {
        let cpu = CpuDistance::new(DistanceMetric::Hamming);
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![1.0, 1.0, 0.0, 0.0];
        let dist = cpu.distance(&a, &b);
        // 2 bits differ (positions 1 and 2)
        assert!((dist - 2.0).abs() < 1e-5);
    }

    #[test]
    fn test_cpu_distance_jaccard() {
        let cpu = CpuDistance::new(DistanceMetric::Jaccard);
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0];
        // intersection = min(1,1) + min(1,0) + min(0,1) + min(0,0) = 1
        // union = max(1,1) + max(1,0) + max(0,1) + max(0,0) = 3
        // similarity = 1/3, distance = 2/3
        let dist = cpu.distance(&a, &b);
        let expected = 1.0 - (1.0 / 3.0);
        assert!((dist - expected).abs() < 1e-5);
    }

    #[test]
    fn test_cpu_distance_metric_accessor() {
        let cpu = CpuDistance::new(DistanceMetric::Euclidean);
        assert_eq!(cpu.metric(), DistanceMetric::Euclidean);
    }

    #[test]
    fn test_simd_distance_metric_accessor() {
        let simd = SimdDistance::new(DistanceMetric::Cosine);
        assert_eq!(simd.metric(), DistanceMetric::Cosine);
    }

    #[test]
    fn test_native_simd_metric_accessor() {
        let native = NativeSimdDistance::new(DistanceMetric::DotProduct);
        assert_eq!(native.metric(), DistanceMetric::DotProduct);
    }

    #[test]
    fn test_simd_dot_product() {
        let simd = SimdDistance::new(DistanceMetric::DotProduct);
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![1.0, 1.0, 1.0, 1.0];
        let dist = simd.distance(&a, &b);
        // dot = 10, distance = -10
        assert!((dist + 10.0).abs() < 1e-4);
    }

    #[test]
    fn test_simd_euclidean() {
        let simd = SimdDistance::new(DistanceMetric::Euclidean);
        let a = vec![0.0, 0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0, 0.0];
        let dist = simd.distance(&a, &b);
        assert!((dist - 5.0).abs() < 1e-4);
    }

    #[test]
    fn test_native_simd_hamming() {
        let native = NativeSimdDistance::new(DistanceMetric::Hamming);
        let a: Vec<f32> = (0..32)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..32)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let dist = native.distance(&a, &b);
        assert!(dist >= 0.0);
    }

    #[test]
    fn test_native_simd_jaccard() {
        let native = NativeSimdDistance::new(DistanceMetric::Jaccard);
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 1.0, 1.0, 0.0];
        let dist = native.distance(&a, &b);
        assert!((0.0..=1.0).contains(&dist));
    }

    // =========================================================================
    // Tests for AdaptiveSimdDistance bug fix (returns distance, not similarity)
    // =========================================================================

    #[test]
    fn test_adaptive_simd_cosine_returns_distance() {
        let adaptive = super::AdaptiveSimdDistance::new(DistanceMetric::Cosine);

        // Identical vectors should have distance ~0 (not similarity ~1)
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let dist = adaptive.distance(&v, &v);
        assert!(
            dist.abs() < 1e-4,
            "AdaptiveSimdDistance should return distance ~0 for identical vectors, got {dist}"
        );

        // Opposite vectors should have distance ~2 (not similarity ~-1)
        let opposite: Vec<f32> = v.iter().map(|x| -x).collect();
        let dist_opposite = adaptive.distance(&v, &opposite);
        assert!(
            (dist_opposite - 2.0).abs() < 1e-4,
            "AdaptiveSimdDistance should return distance ~2 for opposite vectors, got {dist_opposite}"
        );
    }

    #[test]
    fn test_adaptive_simd_dot_product_returns_distance() {
        let adaptive = super::AdaptiveSimdDistance::new(DistanceMetric::DotProduct);

        // Positive dot product should give negative distance (lower = better)
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 1.0, 1.0];
        let dist = adaptive.distance(&a, &b);
        // dot = 1*1 + 2*1 + 3*1 = 6, distance = -6
        assert!(
            dist < 0.0,
            "AdaptiveSimdDistance DotProduct should return negative distance, got {dist}"
        );
        assert!(
            (dist + 6.0).abs() < 1e-4,
            "Expected distance ~-6, got {dist}"
        );
    }

    #[test]
    fn test_adaptive_simd_jaccard_returns_distance() {
        let adaptive = super::AdaptiveSimdDistance::new(DistanceMetric::Jaccard);

        // Identical vectors should have distance ~0
        let v: Vec<f32> = (0..32)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let dist = adaptive.distance(&v, &v);
        assert!(
            dist.abs() < 1e-4,
            "AdaptiveSimdDistance Jaccard should return distance ~0 for identical vectors, got {dist}"
        );

        // Jaccard distance should be in [0, 1]
        let b: Vec<f32> = (0..32)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let dist2 = adaptive.distance(&v, &b);
        assert!(
            (0.0..=1.0).contains(&dist2),
            "AdaptiveSimdDistance Jaccard distance should be in [0,1], got {dist2}"
        );
    }

    #[test]
    fn test_adaptive_simd_matches_native_simd() {
        // Ensure AdaptiveSimdDistance returns same results as NativeSimdDistance
        let adaptive = super::AdaptiveSimdDistance::new(DistanceMetric::Cosine);
        let native = NativeSimdDistance::new(DistanceMetric::Cosine);

        let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
        let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

        let adaptive_dist = adaptive.distance(&a, &b);
        let native_dist = native.distance(&a, &b);

        assert!(
            (adaptive_dist - native_dist).abs() < 1e-3,
            "AdaptiveSimdDistance should match NativeSimdDistance: adaptive={adaptive_dist}, native={native_dist}"
        );
    }

    #[test]
    fn test_adaptive_simd_euclidean_returns_distance() {
        let adaptive = super::AdaptiveSimdDistance::new(DistanceMetric::Euclidean);

        let a = vec![0.0, 0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0, 0.0];
        let dist = adaptive.distance(&a, &b);

        assert!(
            (dist - 5.0).abs() < 1e-4,
            "AdaptiveSimdDistance Euclidean should return 5.0 for 3-4-5 triangle, got {dist}"
        );
    }

    #[test]
    fn test_adaptive_simd_hamming_returns_distance() {
        let adaptive = super::AdaptiveSimdDistance::new(DistanceMetric::Hamming);

        let a: Vec<f32> = (0..32)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..32)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();

        let dist = adaptive.distance(&a, &b);
        assert!(
            dist >= 0.0,
            "AdaptiveSimdDistance Hamming distance should be non-negative, got {dist}"
        );
    }

    #[test]
    fn test_native_simd_batch_dot_product() {
        let native = NativeSimdDistance::new(DistanceMetric::DotProduct);
        let query: Vec<f32> = vec![1.0; 16];
        let candidates: Vec<Vec<f32>> = (0..5).map(|i| vec![(i + 1) as f32; 16]).collect();
        let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

        let distances = native.batch_distance(&query, &candidate_refs);
        assert_eq!(distances.len(), 5);
        // Each dot product = 16 * (i+1), distance = -dot
        for (i, &d) in distances.iter().enumerate() {
            let expected = -16.0 * ((i + 1) as f32);
            assert!(
                (d - expected).abs() < 1e-3,
                "i={i}: got {d}, expected {expected}"
            );
        }
    }

    #[test]
    fn test_native_simd_batch_euclidean() {
        let native = NativeSimdDistance::new(DistanceMetric::Euclidean);
        let query = vec![0.0; 8];
        let candidates: Vec<Vec<f32>> = vec![vec![1.0; 8], vec![2.0; 8]];
        let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

        let distances = native.batch_distance(&query, &candidate_refs);
        assert_eq!(distances.len(), 2);
    }

    #[test]
    fn test_cosine_scalar_zero_norm() {
        // Test division by zero case
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let dist = cosine_distance_scalar(&a, &b);
        assert!(
            (dist - 1.0).abs() < 1e-5,
            "Zero norm should return distance 1.0"
        );
    }

    #[test]
    fn test_jaccard_scalar_zero_union() {
        // Test division by zero case
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0];
        let dist = jaccard_distance_scalar(&a, &b);
        assert!(
            (dist - 1.0).abs() < 1e-5,
            "Zero union should return distance 1.0"
        );
    }

    #[test]
    fn test_cpu_batch_distance_default_impl() {
        let cpu = CpuDistance::new(DistanceMetric::Euclidean);
        let query = vec![0.0, 0.0, 0.0];
        let c1 = vec![1.0, 0.0, 0.0];
        let c2 = vec![0.0, 2.0, 0.0];
        let candidates: Vec<&[f32]> = vec![&c1, &c2];

        let distances = cpu.batch_distance(&query, &candidates);
        assert_eq!(distances.len(), 2);
        assert!((distances[0] - 1.0).abs() < 1e-5);
        assert!((distances[1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn test_hamming_scalar_all_same() {
        let a = vec![1.0, 2.0, 3.0];
        let dist = hamming_distance_scalar(&a, &a);
        assert!((dist - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_hamming_scalar_all_different() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let dist = hamming_distance_scalar(&a, &b);
        assert!((dist - 3.0).abs() < 1e-5);
    }
}
