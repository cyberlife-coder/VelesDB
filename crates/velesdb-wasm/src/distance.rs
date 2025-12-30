//! Distance metrics for WASM vector operations.

use crate::simd;

/// Distance metric for vector similarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMetric {
    /// Cosine similarity (higher = more similar)
    Cosine,
    /// Euclidean distance (lower = more similar)
    Euclidean,
    /// Dot product (higher = more similar)
    DotProduct,
    /// Hamming distance (lower = more similar)
    Hamming,
    /// Jaccard similarity (higher = more similar)
    Jaccard,
}

impl DistanceMetric {
    /// Calculates the distance/similarity between two vectors.
    #[inline]
    #[must_use]
    pub fn calculate(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            Self::Cosine => simd::cosine_similarity(a, b),
            Self::Euclidean => simd::euclidean_distance(a, b),
            Self::DotProduct => simd::dot_product(a, b),
            Self::Hamming => simd::hamming_distance(a, b),
            Self::Jaccard => simd::jaccard_similarity(a, b),
        }
    }

    /// Returns true if higher values indicate more similarity.
    #[inline]
    #[must_use]
    pub const fn higher_is_better(&self) -> bool {
        match self {
            Self::Cosine | Self::DotProduct | Self::Jaccard => true,
            Self::Euclidean | Self::Hamming => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let result = DistanceMetric::Cosine.calculate(&a, &a);
        assert!((result - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_euclidean_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let result = DistanceMetric::Euclidean.calculate(&a, &a);
        assert!(result.abs() < 1e-5);
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let result = DistanceMetric::DotProduct.calculate(&a, &b);
        assert!((result - 32.0).abs() < 1e-5);
    }

    #[test]
    fn test_higher_is_better() {
        assert!(DistanceMetric::Cosine.higher_is_better());
        assert!(DistanceMetric::DotProduct.higher_is_better());
        assert!(!DistanceMetric::Euclidean.higher_is_better());
    }
}
