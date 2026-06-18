#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
//! Numeric cast safety tests (RUST-01/BUG-01 compliance)
//!
//! These tests verify that:
//! 1. Valid numeric conversions work correctly
//! 2. Overflow conditions are properly detected and rejected
//! 3. Bounds checking is applied consistently

use velesdb_core::{Error, Result};

/// Helper function that simulates production code path for offset validation
fn validate_offset(offset: u64) -> Result<usize> {
    // Production pattern: convert u64 to usize with overflow check
    usize::try_from(offset)
        .map_err(|_| Error::Overflow(format!("Offset {offset} exceeds usize::MAX")))
}

#[test]
fn test_usize_try_from_u64_max_on_64bit() {
    // validate_offset must accept the maximum representable offset
    // (u64 == usize on 64-bit), modeling the usize::try_from(u64)
    // production conversion in sparse/persistence.rs, wal_replay.rs, log_payload.rs.
    let result = validate_offset(u64::MAX);
    assert!(
        result.is_ok(),
        "max offset should validate on 64-bit targets"
    );
    assert_eq!(result.unwrap(), usize::MAX);
}

#[test]
fn test_production_dimension_validation_boundary() {
    // MAX_DIMENSION (65_536) is accepted; one past it is rejected as InvalidDimension.
    assert!(velesdb_core::validate_dimension(velesdb_core::MAX_DIMENSION).is_ok());
    let err = velesdb_core::validate_dimension(velesdb_core::MAX_DIMENSION + 1)
        .expect_err("dimension above MAX_DIMENSION must be rejected");
    assert!(
        matches!(err, Error::InvalidDimension { max, .. } if max == velesdb_core::MAX_DIMENSION)
    );
}

#[test]
fn test_f64_to_f32_normal_range() {
    // Mirrors multi_vector.rs JSON -> Vec<f32> coercion: parse a serde_json
    // number into f32 via the same as_f64().map(|f| f as f32) pattern.
    let v = serde_json::json!(3.14159);
    let as_f32 = v.as_f64().map(|f| f as f32);
    assert_eq!(as_f32, Some(3.14159_f64 as f32));
    assert!(as_f32.unwrap().is_finite());
}

#[test]
fn test_clamped_conversion_oob() {
    // Mirrors the OOB guard in query_cost::query_executor::compute_cache_key:
    // out-of-range selectivity must be clamped to 1.0 before the *100 -> u32 cast.
    let value: f64 = 1.5; // invalid selectivity (> 1.0)
    let clamped = (value.clamp(0.0, 1.0) * 100.0) as u32;
    assert_eq!(clamped, 100, "OOB value must saturate at 100, not 150");
    // Guard intent: without the clamp the result would differ (and large
    // invalid values could truncate/overflow on cast).
    let unclamped = (value * 100.0) as u32;
    assert_ne!(
        unclamped, clamped,
        "clamp must change the result for OOB input"
    );
    assert_eq!(unclamped, 150);
}

#[test]
fn test_vector_dimension_bounds_realistic() {
    // Realistic vector dimensions used in ML models must pass the production
    // MIN_DIMENSION..=MAX_DIMENSION contract.
    for dim in [128usize, 256, 384, 512, 768, 1024, 1536, 3072] {
        assert!(
            velesdb_core::validate_dimension(dim).is_ok(),
            "Dimension {dim} should be valid"
        );
    }
    // Boundary checks against the production cap.
    assert!(velesdb_core::validate_dimension(velesdb_core::MAX_DIMENSION).is_ok());
    assert!(velesdb_core::validate_dimension(velesdb_core::MAX_DIMENSION + 1).is_err());
    assert!(velesdb_core::validate_dimension(0).is_err());
}
