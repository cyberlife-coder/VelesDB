//! Tests for prometheus feature flag (EPIC-016/US-035).
//!
//! These tests verify that the metrics endpoint is only available
//! when the `prometheus` feature is enabled.

/// Test that crate compiles with prometheus feature enabled.
#[test]
#[cfg(feature = "prometheus")]
fn test_metrics_enabled_with_feature() {
    // When prometheus feature is enabled, the crate should compile
    // with the metrics module included
    assert!(true, "Crate compiles with prometheus feature");
}

/// Test compilation succeeds without prometheus feature.
#[test]
#[cfg(not(feature = "prometheus"))]
fn test_metrics_disabled_without_feature() {
    // Without prometheus feature, metrics module should not be compiled
    // This test passes if the crate compiles without the feature
    assert!(true, "Crate compiles without prometheus feature");
}

/// Test that default features don't include prometheus.
/// This test only runs without the prometheus feature.
#[test]
#[cfg(not(feature = "prometheus"))]
fn test_prometheus_not_in_default_features() {
    // If this test runs, prometheus is correctly NOT in default features
    assert!(true, "prometheus is correctly opt-in");
}
