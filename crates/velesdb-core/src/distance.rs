//! Distance metrics for vector similarity calculations.
//!
//! # Performance
//!
//! All distance calculations use SIMD-optimized implementations via the `simd` module:
//! - **Cosine**: Single-pass fused algorithm (2.5x faster than naive 3-pass)
//! - **Euclidean**: Loop-unrolled for auto-vectorization (2.1x faster)
//! - **Dot Product**: Loop-unrolled (2x faster)

use crate::simd;
use serde::{Deserialize, Serialize};

/// Distance metric for vector similarity calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceMetric {
    /// Cosine similarity (1 - `cosine_distance`).
    /// Best for normalized vectors, commonly used with text embeddings.
    Cosine,

    /// Euclidean distance (L2 norm).
    /// Best for spatial data and when magnitude matters.
    Euclidean,

    /// Dot product (inner product).
    /// Best for maximum inner product search (MIPS).
    DotProduct,
}

impl DistanceMetric {
    /// Calculates the distance between two vectors using the specified metric.
    ///
    /// # Arguments
    ///
    /// * `a` - First vector
    /// * `b` - Second vector
    ///
    /// # Returns
    ///
    /// Distance value (lower is more similar for Euclidean, higher for Cosine/DotProduct).
    ///
    /// # Panics
    ///
    /// Panics if vectors have different dimensions.
    ///
    /// # Performance
    ///
    /// Uses SIMD-optimized implementations. Typical latencies for 768d vectors:
    /// - Cosine: ~300ns
    /// - Euclidean: ~135ns
    /// - Dot Product: ~128ns
    #[must_use]
    #[inline]
    pub fn calculate(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            Self::Cosine => simd::cosine_similarity_fast(a, b),
            Self::Euclidean => simd::euclidean_distance_fast(a, b),
            Self::DotProduct => simd::dot_product_fast(a, b),
        }
    }

    /// Returns whether higher values indicate more similarity.
    #[must_use]
    pub const fn higher_is_better(&self) -> bool {
        match self {
            Self::Cosine | Self::DotProduct => true,
            Self::Euclidean => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let similarity = DistanceMetric::Cosine.calculate(&a, &b);
        assert!((similarity - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        let similarity = DistanceMetric::Cosine.calculate(&a, &c);
        assert!(similarity.abs() < 1e-6);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        let distance = DistanceMetric::Euclidean.calculate(&a, &b);
        assert!((distance - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let product = DistanceMetric::DotProduct.calculate(&a, &b);
        assert!((product - 32.0).abs() < 1e-6);
    }
}
