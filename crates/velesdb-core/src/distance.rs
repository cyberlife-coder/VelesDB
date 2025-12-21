//! Distance metrics for vector similarity calculations.
//!
//! # Performance
//!
//! All distance calculations use explicit SIMD implementations via the `simd_explicit` module:
//! - **Cosine**: Single-pass fused SIMD (4x faster than auto-vectorized)
//! - **Euclidean**: Explicit f32x8 SIMD (2.8x faster)
//! - **Dot Product**: Explicit f32x8 SIMD (3x faster)
//! - **Hamming (binary)**: POPCNT on packed u64 (48x faster than f32)

use crate::simd_explicit;
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

    /// Hamming distance for binary vectors.
    /// Counts the number of positions where bits differ.
    /// Best for binary embeddings and locality-sensitive hashing.
    Hamming,

    /// Jaccard similarity for set-like vectors.
    /// Measures intersection over union of non-zero elements.
    /// Best for sparse vectors, tags, and set membership.
    Jaccard,
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
            Self::Cosine => simd_explicit::cosine_similarity_simd(a, b),
            Self::Euclidean => simd_explicit::euclidean_distance_simd(a, b),
            Self::DotProduct => simd_explicit::dot_product_simd(a, b),
            Self::Hamming => simd_explicit::hamming_distance_simd(a, b),
            Self::Jaccard => simd_explicit::jaccard_similarity_simd(a, b),
        }
    }

    /// Returns whether higher values indicate more similarity.
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

    #[test]
    fn test_higher_is_better() {
        // Cosine: higher similarity = more similar
        assert!(DistanceMetric::Cosine.higher_is_better());

        // DotProduct: higher product = more similar
        assert!(DistanceMetric::DotProduct.higher_is_better());

        // Euclidean: lower distance = more similar
        assert!(!DistanceMetric::Euclidean.higher_is_better());
    }

    #[test]
    fn test_metric_serialization() {
        // Test that metrics can be serialized/deserialized
        let metric = DistanceMetric::Cosine;
        let json = serde_json::to_string(&metric).unwrap();
        let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(metric, deserialized);

        let metric = DistanceMetric::Euclidean;
        let json = serde_json::to_string(&metric).unwrap();
        let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(metric, deserialized);

        let metric = DistanceMetric::DotProduct;
        let json = serde_json::to_string(&metric).unwrap();
        let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(metric, deserialized);

        let metric = DistanceMetric::Hamming;
        let json = serde_json::to_string(&metric).unwrap();
        let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(metric, deserialized);

        let metric = DistanceMetric::Jaccard;
        let json = serde_json::to_string(&metric).unwrap();
        let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(metric, deserialized);
    }

    // =========================================================================
    // TDD Tests for Hamming Distance (WIS-33)
    // =========================================================================

    #[test]
    fn test_hamming_distance_identical() {
        // Identical binary vectors should have distance 0
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0];
        let distance = DistanceMetric::Hamming.calculate(&a, &b);
        assert!(
            (distance - 0.0).abs() < 1e-6,
            "Identical vectors: distance = 0"
        );
    }

    #[test]
    fn test_hamming_distance_completely_different() {
        // Completely different binary vectors
        let a = vec![1.0, 1.0, 1.0, 1.0];
        let b = vec![0.0, 0.0, 0.0, 0.0];
        let distance = DistanceMetric::Hamming.calculate(&a, &b);
        assert!(
            (distance - 4.0).abs() < 1e-6,
            "All bits differ: distance = 4"
        );
    }

    #[test]
    fn test_hamming_distance_partial() {
        // Some bits differ
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![1.0, 1.0, 0.0, 0.0];
        let distance = DistanceMetric::Hamming.calculate(&a, &b);
        assert!((distance - 2.0).abs() < 1e-6, "2 bits differ: distance = 2");
    }

    #[test]
    fn test_hamming_higher_is_better() {
        // Hamming: lower distance = more similar
        assert!(!DistanceMetric::Hamming.higher_is_better());
    }

    // =========================================================================
    // TDD Tests for Jaccard Similarity (WIS-33)
    // =========================================================================

    #[test]
    fn test_jaccard_similarity_identical() {
        // Identical sets should have similarity 1.0
        let a = vec![1.0, 0.0, 1.0, 1.0];
        let b = vec![1.0, 0.0, 1.0, 1.0];
        let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
        assert!(
            (similarity - 1.0).abs() < 1e-6,
            "Identical sets: similarity = 1.0"
        );
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        // Disjoint sets should have similarity 0.0
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 1.0, 1.0];
        let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
        assert!(
            (similarity - 0.0).abs() < 1e-6,
            "Disjoint sets: similarity = 0.0"
        );
    }

    #[test]
    fn test_jaccard_similarity_partial_overlap() {
        // Partial overlap: intersection=2, union=4, similarity=0.5
        let a = vec![1.0, 1.0, 1.0, 0.0];
        let b = vec![1.0, 1.0, 0.0, 1.0];
        let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
        assert!(
            (similarity - 0.5).abs() < 1e-6,
            "Partial overlap: similarity = 0.5"
        );
    }

    #[test]
    fn test_jaccard_similarity_empty_sets() {
        // Both empty sets - defined as 1.0 (identical)
        let a = vec![0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0, 0.0];
        let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
        assert!(
            (similarity - 1.0).abs() < 1e-6,
            "Empty sets: similarity = 1.0"
        );
    }

    #[test]
    fn test_jaccard_higher_is_better() {
        // Jaccard: higher similarity = more similar
        assert!(DistanceMetric::Jaccard.higher_is_better());
    }
}
