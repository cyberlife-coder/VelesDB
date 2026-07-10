//! Unit tests for the shared-executor conformance golden cases.
//!
//! Feature: core-control-plane-boundary
//! Validates: Requirements 7.3, 7.5

use super::{check_executor, executor_reference_vectors};
use crate::hash_edge_id;

/// Req 7.3 — core agrees with its own frozen golden table: running the real
/// core reference operation (`hash_edge_id`) against the frozen executor
/// vectors yields NO divergences.
#[test]
fn core_reference_matches_frozen_golden_table() {
    let divergences = check_executor(hash_edge_id);
    assert!(
        divergences.is_empty(),
        "core hash_edge_id diverged from its own golden table: {divergences:?}"
    );
}

/// Req 7.3 — every frozen golden `expected` value is pinned to the real
/// derivation, so the table cannot silently drift from `hash_edge_id`.
#[test]
fn golden_expected_values_pin_to_real_derivation() {
    for vector in executor_reference_vectors() {
        let derived = hash_edge_id(vector.source, vector.target, vector.label);
        assert_eq!(
            derived, vector.expected,
            "frozen expected for ({}, {}, {:?}) does not match hash_edge_id",
            vector.source, vector.target, vector.label
        );
    }
}

/// Req 7.5 — the harness is runnable against an injected CANDIDATE
/// implementation. A candidate that reproduces the reference operation is
/// reported as fully conformant (empty divergence set).
#[test]
fn agreeing_candidate_reports_no_divergence() {
    // Candidate closure delegating to the reference derivation.
    let candidate = |source: u64, target: u64, label: &str| hash_edge_id(source, target, label);
    let divergences = check_executor(candidate);
    assert!(
        divergences.is_empty(),
        "an agreeing candidate must report no divergence, got: {divergences:?}"
    );
}

/// Req 7.5 — a divergent candidate is detected, and every reference case is
/// reported (a constant output disagrees with all frozen outputs that differ
/// from the constant).
#[test]
fn constant_candidate_is_detected_as_divergent() {
    // A candidate that always returns 0 diverges on every non-zero golden case.
    let candidate = |_s: u64, _t: u64, _l: &str| 0_u64;
    let divergences = check_executor(candidate);
    assert!(
        !divergences.is_empty(),
        "a constant candidate must be detected as divergent"
    );
    // Only the all-zero case (source=0, target=0, label="") has a non-zero
    // golden id; every other case must be flagged. Confirm the reported count
    // matches the number of vectors whose expected id is not zero.
    let expected_divergent = executor_reference_vectors()
        .into_iter()
        .filter(|v| v.expected != 0)
        .count();
    assert_eq!(
        divergences.len(),
        expected_divergent,
        "constant candidate should diverge on every case with a non-zero golden id"
    );
}

/// Req 7.5 — a candidate that diverges on a SINGLE input is detected, and the
/// divergence entry identifies exactly that case with both the expected golden
/// output and the candidate's actual output.
#[test]
fn single_case_divergence_identifies_the_specific_case() {
    let vectors = executor_reference_vectors();
    let target_case = vectors
        .first()
        .copied()
        .expect("reference table must be non-empty");

    // Candidate agrees everywhere except it flips the low bit for the exact
    // (source, target, label) of the first reference case.
    let candidate = |source: u64, target: u64, label: &str| {
        let base = hash_edge_id(source, target, label);
        if source == target_case.source
            && target == target_case.target
            && label == target_case.label
        {
            base ^ 1
        } else {
            base
        }
    };

    let divergences = check_executor(candidate);
    assert_eq!(
        divergences.len(),
        1,
        "exactly one case should diverge, got: {divergences:?}"
    );
    let divergence = &divergences[0];
    let expected_case = format!(
        "hash_edge_id({}, {}, {:?})",
        target_case.source, target_case.target, target_case.label
    );
    assert_eq!(divergence.case, expected_case);
    assert_eq!(
        divergence.expected,
        format!("{:#018x}", target_case.expected)
    );
    assert_eq!(
        divergence.actual,
        format!("{:#018x}", target_case.expected ^ 1)
    );
}
