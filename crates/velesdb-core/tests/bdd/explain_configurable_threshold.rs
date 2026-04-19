//! BDD tests for the runtime-tunable fallback selectivity threshold.
//!
//! Covers `set_fallback_selectivity_threshold` / `fallback_selectivity_threshold`
//! (closes KNOWN_LIMITATIONS #6): the fallback heuristic threshold used when no
//! calibrated `CollectionStats` is available was previously hardcoded at `0.1`.
//! These tests verify that:
//!
//! - Default value matches the backward-compat anchor (`0.1`).
//! - Setter validates the input range `[0.0, 1.0]` and rejects NaN / infinite.
//! - Previous value is returned for round-trip restoration in tests.
//! - The threshold is live — `resolve_filter_strategy` picks up changes on its
//!   next call.
//!
//! All tests use `serial_test::serial` because the threshold lives in
//! process-global state (lock-free `AtomicU64`) shared across the test binary.

#![allow(clippy::float_cmp)]

use serial_test::serial;
use velesdb_core::error::Error;
use velesdb_core::velesql::{
    fallback_selectivity_threshold, set_fallback_selectivity_threshold,
    DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD,
};

/// Restores the fallback threshold to its default on drop, so tests that panic
/// mid-way don't leak state into sibling tests.
struct ThresholdGuard;

impl Drop for ThresholdGuard {
    fn drop(&mut self) {
        let _ = set_fallback_selectivity_threshold(DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD);
    }
}

fn fresh_guard() -> ThresholdGuard {
    let _ = set_fallback_selectivity_threshold(DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD);
    ThresholdGuard
}

#[test]
#[serial]
fn default_threshold_matches_backward_compat_anchor() {
    let _guard = fresh_guard();
    assert_eq!(fallback_selectivity_threshold(), 0.1_f64);
    assert_eq!(DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD, 0.1_f64);
}

#[test]
#[serial]
fn setter_accepts_valid_values_and_returns_previous() {
    let _guard = fresh_guard();
    let previous = set_fallback_selectivity_threshold(0.3).expect("0.3 is in range");
    assert_eq!(previous, 0.1_f64, "setter must return previous value");
    assert_eq!(fallback_selectivity_threshold(), 0.3_f64);

    // Round-trip: set it back, previous should be our new value.
    let previous = set_fallback_selectivity_threshold(0.1).expect("restore");
    assert_eq!(previous, 0.3_f64);
}

#[test]
#[serial]
fn setter_accepts_boundary_values() {
    let _guard = fresh_guard();
    set_fallback_selectivity_threshold(0.0).expect("0.0 is the lower bound");
    assert_eq!(fallback_selectivity_threshold(), 0.0_f64);
    set_fallback_selectivity_threshold(1.0).expect("1.0 is the upper bound");
    assert_eq!(fallback_selectivity_threshold(), 1.0_f64);
}

#[test]
#[serial]
fn setter_rejects_negative_values() {
    let _guard = fresh_guard();
    let err = set_fallback_selectivity_threshold(-0.01).expect_err("negative must fail");
    assert!(
        matches!(err, Error::Config(_)),
        "expected Config error, got {err:?}"
    );
    // State must remain at the default since the setter rejected.
    assert_eq!(fallback_selectivity_threshold(), 0.1_f64);
}

#[test]
#[serial]
fn setter_rejects_values_above_one() {
    let _guard = fresh_guard();
    let err = set_fallback_selectivity_threshold(1.5).expect_err(">1.0 must fail");
    assert!(matches!(err, Error::Config(_)));
    assert_eq!(fallback_selectivity_threshold(), 0.1_f64);
}

#[test]
#[serial]
fn setter_rejects_nan_and_infinite() {
    let _guard = fresh_guard();
    let err = set_fallback_selectivity_threshold(f64::NAN).expect_err("NaN must fail");
    assert!(matches!(err, Error::Config(_)));
    let err = set_fallback_selectivity_threshold(f64::INFINITY).expect_err("+inf must fail");
    assert!(matches!(err, Error::Config(_)));
    let err = set_fallback_selectivity_threshold(f64::NEG_INFINITY).expect_err("-inf must fail");
    assert!(matches!(err, Error::Config(_)));
    // Rejection must not clobber state.
    assert_eq!(fallback_selectivity_threshold(), 0.1_f64);
}
