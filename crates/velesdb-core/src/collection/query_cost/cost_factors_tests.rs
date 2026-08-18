use super::*;

#[test]
fn test_ssd_optimized_factors() {
    let factors = OperationCostFactors::ssd_optimized();
    assert!(factors.random_page_cost < OperationCostFactors::default().random_page_cost);
}

#[test]
fn test_hdd_optimized_factors() {
    let factors = OperationCostFactors::hdd_optimized();
    assert!(factors.random_page_cost > OperationCostFactors::default().random_page_cost);
}

#[test]
fn test_in_memory_factors() {
    let factors = OperationCostFactors::in_memory();
    assert!(factors.seq_page_cost < OperationCostFactors::default().seq_page_cost);
    assert!(factors.random_page_cost < OperationCostFactors::default().random_page_cost);
}

#[test]
fn test_is_default() {
    assert!(OperationCostFactors::default().is_default());
    assert!(!OperationCostFactors::ssd_optimized().is_default());
}

#[test]
fn test_clamped_within_bounds() {
    let extreme = OperationCostFactors {
        seq_page_cost: 999.0,
        random_page_cost: -1.0,
        cpu_tuple_cost: 0.0,
        cpu_index_cost: 100.0,
        cpu_distance_cost: 0.0,
        cpu_edge_cost: 0.0,
    };
    let clamped = extreme.clamped();
    assert!((clamped.seq_page_cost - CostFactorBounds::SEQ_PAGE_COST.1).abs() < f64::EPSILON);
    assert!((clamped.random_page_cost - CostFactorBounds::RANDOM_PAGE_COST.0).abs() < f64::EPSILON);
    assert!((clamped.cpu_tuple_cost - CostFactorBounds::CPU_TUPLE_COST.0).abs() < f64::EPSILON);
    assert!((clamped.cpu_index_cost - CostFactorBounds::CPU_INDEX_COST.1).abs() < f64::EPSILON);
}

#[test]
fn test_clamp_with_log_no_change() {
    let result = clamp_with_log("test", 5.0, (1.0, 10.0));
    assert!((result - 5.0).abs() < f64::EPSILON);
}

#[test]
fn test_clamp_with_log_clamps_low() {
    let result = clamp_with_log("test", -1.0, (0.0, 10.0));
    assert!(result.abs() < f64::EPSILON);
}

#[test]
fn test_clamp_with_log_clamps_high() {
    let result = clamp_with_log("test", 99.0, (0.0, 10.0));
    assert!((result - 10.0).abs() < f64::EPSILON);
}
