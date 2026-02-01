//! Distance metrics for vector similarity calculations.
//!
//! # Performance
//!
//! All distance calculations use direct SIMD dispatch via `simd_native` module,
//! eliminating intermediate dispatch overhead for maximum performance:
//! - **Cosine**: Direct AVX-512/AVX2/NEON intrinsics
//! - **Euclidean**: Direct native intrinsics with 4-acc unrolling
//! - **Dot Product**: Direct FMA-optimized intrinsics
//! - **Hamming (binary)**: POPCNT on packed u64 (48x faster than f32)
//! - **Jaccard**: Set similarity with SIMD acceleration

use crate::simd_native;
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
    /// - Cosine: ~32ns
    /// - Euclidean: ~20ns
    /// - Dot Product: ~18ns
    #[must_use]
    #[inline]
    pub fn calculate(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            Self::Cosine => simd_native::cosine_similarity_native(a, b),
            Self::Euclidean => simd_native::euclidean_native(a, b),
            Self::DotProduct => simd_native::dot_product_native(a, b),
            Self::Hamming => simd_native::hamming_distance_native(a, b) as f32,
            Self::Jaccard => simd_native::jaccard_similarity_native(a, b),
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

    /// Sorts search results by distance/similarity according to the metric.
    ///
    /// - **Similarity metrics** (`Cosine`, `DotProduct`, `Jaccard`): sorts descending (higher = better)
    /// - **Distance metrics** (`Euclidean`, `Hamming`): sorts ascending (lower = better)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut results = vec![(1, 0.9), (2, 0.7), (3, 0.8)];
    /// DistanceMetric::Cosine.sort_results(&mut results);
    /// assert_eq!(results[0].0, 1); // Highest similarity first
    /// ```
    pub fn sort_results(&self, results: &mut [(u64, f32)]) {
        if self.higher_is_better() {
            // Similarity metrics: descending order (higher = better)
            results.sort_by(|a, b| b.1.total_cmp(&a.1));
        } else {
            // Distance metrics: ascending order (lower = better)
            results.sort_by(|a, b| a.1.total_cmp(&b.1));
        }
    }
}
