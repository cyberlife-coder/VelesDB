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

/// Helper function that simulates production code path for dimension validation
fn validate_dimension(dimension: usize) -> Result<u32> {
    // Production pattern: convert usize to u32 with overflow check
    u32::try_from(dimension)
        .map_err(|_| Error::Overflow(format!("Dimension {} exceeds u32::MAX", dimension)))
}

/// Helper function that simulates production code path for offset validation
fn validate_offset(offset: u64) -> Result<usize> {
    // Production pattern: convert u64 to usize with overflow check
    usize::try_from(offset)
        .map_err(|_| Error::Overflow(format!("Offset {} exceeds usize::MAX", offset)))
}

/// Helper function that simulates production code path for count validation
fn validate_count(count: usize) -> Result<u32> {
    // Production pattern: convert usize to u32 for serialization
    u32::try_from(count).map_err(|_| Error::Overflow(format!("Count {} exceeds u32::MAX", count)))
}

#[test]
fn test_u32_try_from_usize_valid() {
    let valid: usize = 1000;
    assert_eq!(u32::try_from(valid).unwrap(), 1000);
}

#[test]
fn test_u32_try_from_usize_max() {
    // u32::MAX should convert successfully
    let max_u32: usize = u32::MAX as usize;
    assert_eq!(u32::try_from(max_u32).unwrap(), u32::MAX);
}

#[test]
fn test_u32_try_from_usize_overflow() {
    // u32::MAX + 1 should fail
    let overflow: usize = (u32::MAX as usize) + 1;
    assert!(u32::try_from(overflow).is_err());
}

#[test]
fn test_usize_try_from_u64_valid() {
    let valid: u64 = 1000;
    assert_eq!(usize::try_from(valid).unwrap(), 1000);
}

#[test]
fn test_usize_try_from_u64_max_on_64bit() {
    // On 64-bit systems, usize::MAX equals u64::MAX
    // This test verifies conversion at the boundary
    let max_usize: u64 = usize::MAX as u64;
    assert_eq!(usize::try_from(max_usize).unwrap(), usize::MAX);
}

#[test]
fn test_dimension_validation_valid() {
    let result = validate_dimension(768);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 768);
}

#[test]
fn test_dimension_validation_max() {
    let result = validate_dimension(u32::MAX as usize);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), u32::MAX);
}

#[test]
fn test_dimension_validation_overflow() {
    let oversized = (u32::MAX as usize) + 1;
    let result = validate_dimension(oversized);
    assert!(result.is_err());

    // Verify it's the correct error type
    match result {
        Err(Error::Overflow(_)) => (), // Expected
        _ => panic!("Expected Error::Overflow for oversized dimension"),
    }
}

#[test]
fn test_offset_validation_valid() {
    let result = validate_offset(1024);
    assert!(result.is_ok());
}

#[test]
fn test_offset_validation_large() {
    // Test with a large but valid offset
    let large_offset: u64 = 1024 * 1024 * 1024; // 1GB
    let result = validate_offset(large_offset);
    assert!(result.is_ok());
}

#[test]
fn test_count_validation_valid() {
    let result = validate_count(100);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 100);
}

#[test]
fn test_count_validation_zero() {
    let result = validate_count(0);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn test_i64_to_u64_positive() {
    let positive: i64 = 1000;
    assert_eq!(positive as u64, 1000);
}

#[test]
fn test_i64_to_u64_zero() {
    let zero: i64 = 0;
    assert_eq!(zero as u64, 0);
}

#[test]
fn test_f64_to_f32_precision_loss() {
    // Very large f64 values lose precision when cast to f32
    let large: f64 = 1e300;
    let as_f32 = large as f32;
    assert!(as_f32.is_infinite()); // f32::MAX is ~3.4e38
}

#[test]
fn test_f64_to_f32_normal_range() {
    // Normal values should convert reasonably
    let normal: f64 = 3.14159;
    let as_f32 = normal as f32;
    assert!((as_f32 - 3.14159).abs() < 0.0001);
}

#[test]
fn test_clamped_conversion_valid() {
    // Pattern: clamp then cast for bounded values
    let value: f64 = 0.75;
    let clamped = (value.clamp(0.0, 1.0) * 100.0) as u32;
    assert_eq!(clamped, 75);
}

#[test]
fn test_clamped_conversion_oob() {
    // Values outside [0.0, 1.0] are clamped
    let value: f64 = 1.5;
    let clamped = (value.clamp(0.0, 1.0) * 100.0) as u32;
    assert_eq!(clamped, 100);
}

#[test]
fn test_vector_dimension_bounds_realistic() {
    // Test realistic vector dimensions used in ML models
    let dimensions = vec![128, 256, 384, 512, 768, 1024, 1536, 3072];

    for dim in dimensions {
        let result = validate_dimension(dim);
        assert!(result.is_ok(), "Dimension {} should be valid", dim);
    }
}

#[test]
fn test_batch_size_validation() {
    // Test batch size limits for bulk operations
    let small_batch = validate_count(1);
    assert!(small_batch.is_ok());

    let medium_batch = validate_count(1000);
    assert!(medium_batch.is_ok());

    let large_batch = validate_count(100_000);
    assert!(large_batch.is_ok());
}

#[test]
fn test_error_message_contains_value() {
    let oversized = (u32::MAX as usize) + 1;
    let result = validate_dimension(oversized);

    match result {
        Err(Error::Overflow(msg)) => {
            assert!(
                msg.contains("Dimension"),
                "Error message should mention 'Dimension'"
            );
            assert!(
                msg.contains(&oversized.to_string()),
                "Error message should contain the value"
            );
        }
        _ => panic!("Expected Error::Overflow with descriptive message"),
    }
}
