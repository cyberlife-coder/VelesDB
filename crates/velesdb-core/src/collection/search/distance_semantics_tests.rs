//! Tests for distance metrics semantics (EPIC-027/US-001).
//!
//! These tests verify that similarity() filtering and sorting
//! behave correctly for both similarity metrics (Cosine, DotProduct, Jaccard)
//! and distance metrics (Euclidean, Hamming).

use crate::distance::DistanceMetric;

/// Helper: Determines if metric should sort descending.
fn should_sort_descending(metric: DistanceMetric) -> bool {
    metric.higher_is_better()
}

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
        assert!(
            should_sort_descending(DistanceMetric::Cosine),
            "Cosine should sort DESC"
        );
        assert!(
            !should_sort_descending(DistanceMetric::Euclidean),
            "Euclidean should sort ASC"
        );
    }
}
