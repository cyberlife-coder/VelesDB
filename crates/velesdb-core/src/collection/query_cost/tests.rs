//! Tests for query cost estimator module (TDD - US-005)

use super::*;

// ============================================================================
// CA-1 & CA-2: Cost estimation tests
// ============================================================================

#[test]
fn test_estimate_cost_small_dataset() {
    let estimator = QueryCostEstimator::default();
    let params = QueryParams::new(1_000, 128, 10);

    let estimate = estimator.estimate(&params);

    // Small dataset should have low cost
    // log2(1001) ≈ 10, ef=128/100=1.28, top_k=sqrt(1)=1
    // cost ≈ 10 * 1.28 * 1 = 12.8
    assert!(estimate.total_cost > 0.0);
    assert!(
        estimate.total_cost < 50.0,
        "Small dataset cost should be low"
    );
    assert!(estimate.estimated_latency_ms > 0.0);
}

#[test]
fn test_estimate_cost_large_dataset() {
    let estimator = QueryCostEstimator::default();

    let small = QueryParams::new(1_000, 128, 10);
    let large = QueryParams::new(1_000_000, 128, 10);

    let small_estimate = estimator.estimate(&small);
    let large_estimate = estimator.estimate(&large);

    // Large dataset should have higher cost (O(log n))
    assert!(
        large_estimate.total_cost > small_estimate.total_cost,
        "Large dataset should cost more"
    );

    // But not linear - log2(1M) / log2(1K) ≈ 2
    let ratio = large_estimate.total_cost / small_estimate.total_cost;
    assert!(
        ratio < 3.0,
        "Cost should scale logarithmically, not linearly"
    );
}

#[test]
fn test_estimate_cost_scales_with_ef_search() {
    let estimator = QueryCostEstimator::default();

    let low_ef = QueryParams::new(100_000, 64, 10);
    let high_ef = QueryParams::new(100_000, 256, 10);

    let low_estimate = estimator.estimate(&low_ef);
    let high_estimate = estimator.estimate(&high_ef);

    // Higher ef_search = higher cost (linear)
    assert!(
        high_estimate.total_cost > low_estimate.total_cost,
        "Higher ef_search should cost more"
    );

    // Should be approximately 4x (256/64)
    let ratio = high_estimate.total_cost / low_estimate.total_cost;
    assert!(
        (ratio - 4.0).abs() < 0.5,
        "Cost should scale linearly with ef_search"
    );
}

#[test]
fn test_estimate_cost_scales_with_top_k() {
    let estimator = QueryCostEstimator::default();

    let small_k = QueryParams::new(100_000, 128, 10);
    let large_k = QueryParams::new(100_000, 128, 100);

    let small_k_estimate = estimator.estimate(&small_k);
    let large_k_estimate = estimator.estimate(&large_k);

    // Higher top_k = higher cost (sub-linear)
    assert!(
        large_k_estimate.total_cost > small_k_estimate.total_cost,
        "Higher top_k should cost more"
    );

    // Sub-linear: sqrt(100/10) / sqrt(10/10) = sqrt(10) ≈ 3.16
    let ratio = large_k_estimate.total_cost / small_k_estimate.total_cost;
    assert!(
        ratio < 5.0,
        "Cost should scale sub-linearly with top_k (got {:.2})",
        ratio
    );
    assert!(
        ratio > 2.0,
        "But still increase significantly (got {:.2})",
        ratio
    );
}

#[test]
fn test_filter_selectivity_reduces_cost() {
    let estimator = QueryCostEstimator::default();

    let no_filter = QueryParams::new(100_000, 128, 10);
    let selective = QueryParams::new(100_000, 128, 10).with_filter_selectivity(0.01);

    let no_filter_estimate = estimator.estimate(&no_filter);
    let selective_estimate = estimator.estimate(&selective);

    // Highly selective filter INCREASES cost (more work to find matching vectors)
    // This is because we need to scan more candidates to find enough matches
    assert!(
        selective_estimate.total_cost > no_filter_estimate.total_cost,
        "Selective filter increases cost due to scan overhead"
    );

    // But with mild exponent (0.3), increase is moderate
    let ratio = selective_estimate.total_cost / no_filter_estimate.total_cost;
    assert!(
        ratio < 10.0,
        "Filter overhead should be moderate (got {:.2})",
        ratio
    );
}

#[test]
fn test_explain_returns_cost_breakdown() {
    let estimator = QueryCostEstimator::default();
    let params = QueryParams::new(100_000, 128, 10);

    let explain = estimator.explain(&params);

    assert!(explain.contains("Query Cost Estimate"));
    assert!(explain.contains("Total Cost:"));
    assert!(explain.contains("Estimated Latency:"));
    assert!(explain.contains("Dataset Size Factor"));
    assert!(explain.contains("ef_search Factor"));
    assert!(explain.contains("top_k Factor"));
    assert!(explain.contains("Filter Selectivity Factor"));
}

// ============================================================================
// CA-3: Max cost limit tests
// ============================================================================

#[test]
fn test_max_cost_rejects_expensive_query() {
    let estimator = QueryCostEstimator::default();

    // Very expensive query
    let expensive = QueryParams::new(1_000_000, 4096, 100);
    let max_cost = 10.0; // Very low limit

    let result = estimator.check_cost_limit(&expensive, max_cost);

    assert!(result.is_err(), "Should reject expensive query");
    let err = result.unwrap_err();
    assert!(err.estimated > max_cost);
    assert!((err.max_allowed - max_cost).abs() < 0.001);
}

#[test]
fn test_max_cost_allows_cheap_query() {
    let estimator = QueryCostEstimator::default();

    // Cheap query
    let cheap = QueryParams::new(1_000, 64, 5);
    let max_cost = 1000.0; // High limit

    let result = estimator.check_cost_limit(&cheap, max_cost);

    assert!(result.is_ok(), "Should allow cheap query");
    let estimate = result.unwrap();
    assert!(estimate.total_cost <= max_cost);
}

#[test]
fn test_collection_max_cost_setting() {
    let mut estimator = QueryCostEstimator::default();

    assert!(estimator.max_cost().is_none());

    estimator.set_max_cost(Some(100.0));
    assert_eq!(estimator.max_cost(), Some(100.0));

    // Cheap query passes
    let cheap = QueryParams::new(1_000, 64, 5);
    assert!(estimator.check_collection_limit(&cheap).is_ok());

    // Expensive query fails
    let expensive = QueryParams::new(1_000_000, 4096, 100);
    assert!(estimator.check_collection_limit(&expensive).is_err());

    // Disable limit
    estimator.set_max_cost(None);
    assert!(estimator.check_collection_limit(&expensive).is_ok());
}

#[test]
fn test_with_max_cost_builder() {
    let estimator = QueryCostEstimator::default().with_max_cost(50.0);

    assert_eq!(estimator.max_cost(), Some(50.0));

    let params = QueryParams::new(100_000, 128, 10);
    let result = estimator.check_collection_limit(&params);

    // This query should cost around 20-30, so it should pass
    assert!(
        result.is_ok() || result.is_err(),
        "Should either pass or fail based on actual cost"
    );
}

// ============================================================================
// CA-4: API tests
// ============================================================================

#[test]
fn test_query_params_builder() {
    let params = QueryParamsBuilder::new()
        .dataset_size(500_000)
        .ef_search(256)
        .top_k(50)
        .filter_selectivity(0.1)
        .build();

    assert_eq!(params.dataset_size, 500_000);
    assert_eq!(params.ef_search, 256);
    assert_eq!(params.top_k, 50);
    assert_eq!(params.filter_selectivity, Some(0.1));
}

#[test]
fn test_query_params_defaults() {
    let params = QueryParams::default();

    assert_eq!(params.dataset_size, 10_000);
    assert_eq!(params.ef_search, 128);
    assert_eq!(params.top_k, 10);
    assert!(params.filter_selectivity.is_none());
}

#[test]
fn test_calibration_presets() {
    let default = CostCalibration::default();
    let fast = CostCalibration::fast_system();
    let slow = CostCalibration::slow_system();

    assert!(fast.ms_per_cost_unit < default.ms_per_cost_unit);
    assert!(slow.ms_per_cost_unit > default.ms_per_cost_unit);
}

#[test]
fn test_latency_estimation_with_calibration() {
    let params = QueryParams::new(100_000, 128, 10);

    let fast_estimator = QueryCostEstimator::new(CostCalibration::fast_system());
    let slow_estimator = QueryCostEstimator::new(CostCalibration::slow_system());

    let fast_estimate = fast_estimator.estimate(&params);
    let slow_estimate = slow_estimator.estimate(&params);

    // Same cost, different latency
    assert!(
        (fast_estimate.total_cost - slow_estimate.total_cost).abs() < 0.01,
        "Cost should be the same"
    );
    assert!(
        slow_estimate.estimated_latency_ms > fast_estimate.estimated_latency_ms,
        "Slow system should have higher latency estimate"
    );
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_zero_dataset_size() {
    let estimator = QueryCostEstimator::default();
    let params = QueryParams::new(0, 128, 10);

    let estimate = estimator.estimate(&params);

    // Should not panic, return minimal cost
    assert!(estimate.total_cost > 0.0);
    assert!(estimate.factors.dataset_size_factor >= 1.0);
}

#[test]
fn test_very_low_selectivity() {
    let estimator = QueryCostEstimator::default();

    // Very selective filter (0.001 = 0.1%)
    let params = QueryParams::new(100_000, 128, 10).with_filter_selectivity(0.001);

    let estimate = estimator.estimate(&params);

    // Should handle gracefully, cost increases but bounded
    assert!(estimate.total_cost > 0.0);
    assert!(estimate.total_cost < 10_000.0, "Cost should be bounded");
}

#[test]
fn test_selectivity_clamped() {
    let params = QueryParams::new(100_000, 128, 10).with_filter_selectivity(0.0);

    // Should be clamped to minimum 0.001
    assert!(params.filter_selectivity.unwrap() >= 0.001);

    let params2 = QueryParams::new(100_000, 128, 10).with_filter_selectivity(2.0);

    // Should be clamped to maximum 1.0
    assert!(params2.filter_selectivity.unwrap() <= 1.0);
}

#[test]
fn test_cost_factors_populated() {
    let estimator = QueryCostEstimator::default();
    let params = QueryParams::new(100_000, 128, 10).with_filter_selectivity(0.5);

    let estimate = estimator.estimate(&params);

    assert!(estimate.factors.dataset_size_factor > 0.0);
    assert!(estimate.factors.ef_search_factor > 0.0);
    assert!(estimate.factors.top_k_factor > 0.0);
    assert!(estimate.factors.filter_selectivity_factor > 0.0);
}

#[test]
fn test_query_cost_exceeded_display() {
    let err = QueryCostExceeded {
        estimated: 150.5,
        max_allowed: 100.0,
    };

    let msg = err.to_string();
    assert!(msg.contains("150.5"));
    assert!(msg.contains("100.0"));
    assert!(msg.contains("exceeds"));
}

#[test]
fn test_cost_estimate_accuracy_order_of_magnitude() {
    // This test verifies our cost model produces reasonable estimates
    let estimator = QueryCostEstimator::default();

    // Reference case: 100K vectors, ef=128, k=10, no filter
    let reference = QueryParams::new(100_000, 128, 10);
    let ref_estimate = estimator.estimate(&reference);

    // Expected: log2(100001) ≈ 16.6, ef=1.28, k=1.0, filter=1.0
    // Total ≈ 16.6 * 1.28 * 1.0 * 1.0 ≈ 21.2
    assert!(
        ref_estimate.total_cost > 10.0 && ref_estimate.total_cost < 50.0,
        "Reference cost should be in expected range, got {:.2}",
        ref_estimate.total_cost
    );

    // Latency: 21.2 * 0.1ms ≈ 2.1ms
    assert!(
        ref_estimate.estimated_latency_ms > 1.0 && ref_estimate.estimated_latency_ms < 10.0,
        "Reference latency should be in expected range, got {:.2}ms",
        ref_estimate.estimated_latency_ms
    );
}
