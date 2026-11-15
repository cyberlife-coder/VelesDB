use super::*;

#[test]
fn a_similarity_metric_compares_the_score_as_written() {
    assert!(similarity_holds(CompareOp::Gt, 0.9, 0.5, true));
    assert!(!similarity_holds(CompareOp::Gt, 0.4, 0.5, true));
    assert!(similarity_holds(CompareOp::Lte, 0.5, 0.5, true));
}

/// Under a distance, "more similar than 0.5" is "closer than 0.5".
#[test]
fn a_distance_metric_reads_more_similar_as_closer() {
    assert!(similarity_holds(CompareOp::Gt, 0.2, 0.5, false));
    assert!(!similarity_holds(CompareOp::Gt, 0.9, 0.5, false));
    assert!(similarity_holds(CompareOp::Lt, 0.9, 0.5, false));
    assert!(similarity_holds(CompareOp::Gte, 0.5, 0.5, false));
}

#[test]
fn equality_allows_the_tolerance_under_every_metric() {
    for higher_is_better in [true, false] {
        assert!(similarity_holds(
            CompareOp::Eq,
            0.8004,
            0.8,
            higher_is_better
        ));
        assert!(!similarity_holds(
            CompareOp::Eq,
            0.802,
            0.8,
            higher_is_better
        ));
        assert!(similarity_holds(
            CompareOp::NotEq,
            0.802,
            0.8,
            higher_is_better
        ));
    }
}

/// The threshold is the one the query wrote: `0.8_f32` is above `0.8`.
#[test]
fn the_threshold_is_compared_exactly() {
    assert!(similarity_holds(CompareOp::Gt, 0.8_f32, 0.8, true));
}
