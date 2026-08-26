//! Tests for the adaptive-search escalation rule.

use super::should_escalate;
use crate::distance::DistanceMetric;

/// A cosine tail that merely lands near zero is not a hard query.
///
/// Under a baseline of `min(|first|, |last|)` this reads as a spread of ~91
/// and escalates; measured from cosine's own floor of `-1.0` it is ~0.92 and
/// correctly does not. Every `Adaptive`/`AutoTune` query whose result tail
/// crosses zero would otherwise pay a second traversal.
#[test]
fn a_cosine_tail_near_zero_is_not_a_hard_query() {
    assert!(
        !should_escalate(DistanceMetric::Cosine, 0.9, -0.01),
        "a 0.9 -> -0.01 spread is ordinary for cosine and must not escalate"
    );
    // What the zero-based baseline would have computed, for contrast.
    let naive = (0.9_f32 - -0.01).abs() / 0.9_f32.abs().min(0.01);
    assert!(
        naive > 2.0,
        "the zero-based baseline really would have escalated here ({naive})"
    );
}

/// A genuinely heterogeneous cosine neighbourhood still escalates.
#[test]
fn a_wide_cosine_neighbourhood_escalates() {
    assert!(should_escalate(DistanceMetric::Cosine, 0.95, -0.9));
}

/// A tightly clustered cosine result set does not.
#[test]
fn a_tight_cosine_neighbourhood_does_not_escalate() {
    assert!(!should_escalate(DistanceMetric::Cosine, 0.95, 0.90));
}

/// Unbounded distance metrics keep the original zero-based relative spread.
///
/// Zero *is* the floor for a distance, so nothing changes for these: the ratio
/// is the meaningful quantity because the scale is set by the data.
#[test]
fn unbounded_metrics_keep_the_zero_based_relative_spread() {
    // A 1.0 -> 100.0 Euclidean gap is a 99x ratio: hard.
    assert!(should_escalate(DistanceMetric::Euclidean, 1.0, 100.0));
    // 10.0 -> 11.0 is a 0.1 ratio: easy.
    assert!(!should_escalate(DistanceMetric::Euclidean, 10.0, 11.0));
    assert!(should_escalate(DistanceMetric::Hamming, 1.0, 64.0));
}

/// Jaccard's floor is zero, so its baseline is unchanged by the floor rule.
#[test]
fn jaccard_floor_is_zero_so_its_baseline_is_unchanged() {
    // 0.9 -> 0.1 : baseline 0.1, spread 8 — hard either way.
    assert!(should_escalate(DistanceMetric::Jaccard, 0.9, 0.1));
    // 0.9 -> 0.5 : baseline 0.5, spread 0.8 — easy either way.
    assert!(!should_escalate(DistanceMetric::Jaccard, 0.9, 0.5));
}

/// A degenerate all-equal result set must not divide by zero into an escalation.
#[test]
fn an_all_equal_result_set_does_not_escalate() {
    assert!(!should_escalate(DistanceMetric::Cosine, 0.5, 0.5));
    assert!(!should_escalate(DistanceMetric::Euclidean, 0.0, 0.0));
    // Cosine pinned at its floor: baseline 0, diff 0 — still no escalation.
    assert!(!should_escalate(DistanceMetric::Cosine, -1.0, -1.0));
}
