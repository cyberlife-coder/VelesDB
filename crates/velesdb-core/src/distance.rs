//! Distance metrics for vector similarity calculations.

use serde::{Deserialize, Serialize};

/// Distance metric for vector similarity calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceMetric {
    /// Cosine similarity (1 - cosine_distance).
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
    #[must_use]
    pub fn calculate(&self, a: &[f32], b: &[f32]) -> f32 {
        assert_eq!(a.len(), b.len(), "Vector dimensions must match");

        match self {
            Self::Cosine => Self::cosine_similarity(a, b),
            Self::Euclidean => Self::euclidean_distance(a, b),
            Self::DotProduct => Self::dot_product(a, b),
        }
    }

    /// Calculates cosine similarity between two vectors.
    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot / (norm_a * norm_b)
    }

    /// Calculates Euclidean distance between two vectors.
    fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    /// Calculates dot product between two vectors.
    fn dot_product(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
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
