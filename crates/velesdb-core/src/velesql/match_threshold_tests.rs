//! Tests for the MATCH query-vector threshold rule.

use super::{match_threshold_keeps, validate_match_threshold};

#[test]
fn a_threshold_keeps_rows_at_least_as_similar_whatever_the_metric() {
    // Similarity metric: higher scores are closer.
    assert!(match_threshold_keeps(Some(0.5), 0.5, true));
    assert!(match_threshold_keeps(Some(0.5), 0.9, true));
    assert!(!match_threshold_keeps(Some(0.5), 0.4, true));
    // Distance metric: lower scores are closer.
    assert!(match_threshold_keeps(Some(0.5), 0.5, false));
    assert!(match_threshold_keeps(Some(0.5), 0.1, false));
    assert!(!match_threshold_keeps(Some(0.5), 0.9, false));
    // No threshold keeps every scored row.
    assert!(match_threshold_keeps(None, f32::MIN, true));
}

#[test]
fn a_finite_threshold_with_a_vector_or_no_threshold_is_accepted() {
    let vector = [1.0_f32, 0.0];
    for threshold in [None, Some(0.0), Some(0.5), Some(4.0), Some(-1.0)] {
        validate_match_threshold(Some(&vector), threshold).expect("test: accepted");
    }
    validate_match_threshold(None, None).expect("test: nothing to apply");
}

#[test]
fn a_non_finite_threshold_or_one_without_a_vector_is_refused() {
    let vector = [1.0_f32, 0.0];
    for (query_vector, threshold) in [
        (Some(&vector[..]), f32::NAN),
        (Some(&vector[..]), f32::INFINITY),
        (Some(&vector[..]), f32::NEG_INFINITY),
        (None, f32::NAN),
        (None, 0.5),
    ] {
        let err =
            validate_match_threshold(query_vector, Some(threshold)).expect_err("test: refused");
        assert_eq!(err.code(), "VELES-010", "{err}");
        assert!(err.to_string().contains("threshold"), "{err}");
    }
}
