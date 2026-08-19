use super::{DistanceMetric, DISTANCE_METRIC_NAMES};

/// Forces this test to be revisited whenever a variant is added: the
/// exhaustive `match` (no wildcard arm) fails to compile until the new
/// variant is listed here, which in turn flags the missing const entry.
fn ordinal(metric: DistanceMetric) -> usize {
    match metric {
        DistanceMetric::Cosine => 0,
        DistanceMetric::Euclidean => 1,
        DistanceMetric::DotProduct => 2,
        DistanceMetric::Hamming => 3,
        DistanceMetric::Jaccard => 4,
    }
}

#[test]
fn distance_metric_names_is_exhaustive_and_canonical() {
    let variants = [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ];
    // Tie the variant list to the `ordinal` tripwire so a new variant
    // cannot silently skip this assertion.
    assert_eq!(variants.len(), DISTANCE_METRIC_NAMES.len());
    for (i, variant) in variants.into_iter().enumerate() {
        assert_eq!(ordinal(variant), i);
        assert_eq!(DISTANCE_METRIC_NAMES[i], variant.canonical_name());
    }
}
