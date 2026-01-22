//! Tests for distance metrics semantics (EPIC-027/US-001).
//!
//! These tests verify that similarity() filtering and sorting
//! behave correctly for both similarity metrics (Cosine, DotProduct, Jaccard)
//! and distance metrics (Euclidean, Hamming).

use crate::distance::DistanceMetric;

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that higher_is_better returns correct values for all metrics.
    #[test]
    fn test_higher_is_better_semantics() {
        // Similarity metrics: higher = more similar
        assert!(DistanceMetric::Cosine.higher_is_better());
        assert!(DistanceMetric::DotProduct.higher_is_better());
        assert!(DistanceMetric::Jaccard.higher_is_better());

        // Distance metrics: lower = more similar
        assert!(!DistanceMetric::Euclidean.higher_is_better());
        assert!(!DistanceMetric::Hamming.higher_is_better());
    }

    /// Test sort direction helper for search results.
    #[test]
    fn test_sort_direction_for_metrics() {
        // For similarity metrics (higher=better), sort DESC (highest first)
        // For distance metrics (lower=better), sort ASC (lowest first)
        
        let cosine = DistanceMetric::Cosine;
        let euclidean = DistanceMetric::Euclidean;

        // Helper function that determines sort order
        fn should_sort_descending(metric: &DistanceMetric) -> bool {
            metric.higher_is_better()
        }

        assert!(should_sort_descending(&cosine), "Cosine should sort DESC");
        assert!(!should_sort_descending(&euclidean), "Euclidean should sort ASC");
    }

    /// Test that similarity threshold comparison is metric-aware.
    /// For Cosine: similarity() > 0.8 means "score > 0.8" (more similar)
    /// For Euclidean: similarity() > 0.8 should mean "distance < 0.8" (more similar)
    #[test]
    fn test_threshold_comparison_semantics() {
        // Scores from search results
        let high_cosine_score = 0.95; // Very similar (high = good)
        let low_cosine_score = 0.3;   // Not similar (low = bad)
        
        let low_euclidean_dist = 0.2;  // Very similar (low = good)
        let high_euclidean_dist = 5.0; // Not similar (high = bad)

        let threshold = 0.5;

        // For Cosine (higher_is_better = true):
        // "similarity() > 0.5" should keep high_cosine_score (0.95 > 0.5)
        // and reject low_cosine_score (0.3 < 0.5)
        let cosine = DistanceMetric::Cosine;
        assert!(
            filter_by_similarity_gt(&cosine, high_cosine_score, threshold),
            "High cosine score should pass > threshold"
        );
        assert!(
            !filter_by_similarity_gt(&cosine, low_cosine_score, threshold),
            "Low cosine score should fail > threshold"
        );

        // For Euclidean (higher_is_better = false):
        // "similarity() > 0.5" should mean "more similar than 0.5 distance"
        // which means "distance < 0.5" (inverted comparison)
        let euclidean = DistanceMetric::Euclidean;
        assert!(
            filter_by_similarity_gt(&euclidean, low_euclidean_dist, threshold),
            "Low euclidean distance (0.2) should pass similarity > 0.5"
        );
        assert!(
            !filter_by_similarity_gt(&euclidean, high_euclidean_dist, threshold),
            "High euclidean distance (5.0) should fail similarity > 0.5"
        );
    }

    /// Helper: Filter by "similarity() > threshold" with metric awareness.
    fn filter_by_similarity_gt(metric: &DistanceMetric, score: f32, threshold: f32) -> bool {
        if metric.higher_is_better() {
            // Similarity metric: higher score = more similar
            score > threshold
        } else {
            // Distance metric: lower score = more similar
            // "similarity > threshold" means "distance < threshold"
            score < threshold
        }
    }

    /// Test sorting results by similarity with metric awareness.
    #[test]
    fn test_sort_results_by_similarity() {
        let mut cosine_scores = vec![0.3, 0.9, 0.5, 0.7];
        let mut euclidean_dists = vec![0.3, 0.9, 0.5, 0.7];

        // Sort by similarity (most similar first)
        sort_by_similarity(&DistanceMetric::Cosine, &mut cosine_scores);
        sort_by_similarity(&DistanceMetric::Euclidean, &mut euclidean_dists);

        // Cosine: highest first (0.9, 0.7, 0.5, 0.3)
        assert_eq!(cosine_scores, vec![0.9, 0.7, 0.5, 0.3]);

        // Euclidean: lowest first (0.3, 0.5, 0.7, 0.9)
        assert_eq!(euclidean_dists, vec![0.3, 0.5, 0.7, 0.9]);
    }

    /// Helper: Sort scores by similarity (most similar first).
    fn sort_by_similarity(metric: &DistanceMetric, scores: &mut [f32]) {
        if metric.higher_is_better() {
            // Similarity: sort descending (highest first)
            scores.sort_by(|a, b| b.partial_cmp(a).unwrap());
        } else {
            // Distance: sort ascending (lowest first)
            scores.sort_by(|a, b| a.partial_cmp(b).unwrap());
        }
    }
}
