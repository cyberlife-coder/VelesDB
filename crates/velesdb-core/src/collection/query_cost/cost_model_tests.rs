use super::*;

#[test]
fn test_scan_cost_scales_with_size() {
    let estimator = CostEstimator::default();

    let small = CollectionStats::with_counts(1_000, 0);
    let large = CollectionStats::with_counts(100_000, 0);

    let small_cost = estimator.estimate_scan(&small);
    let large_cost = estimator.estimate_scan(&large);

    assert!(large_cost.total > small_cost.total);
    assert_eq!(small_cost.rows, 1_000);
    assert_eq!(large_cost.rows, 100_000);
}

#[test]
fn test_index_lookup_cheaper_than_scan() {
    let estimator = CostEstimator::default();

    let mut stats = CollectionStats::with_counts(100_000, 0);
    stats.total_size_bytes = 100_000 * 256; // 256 bytes per row

    let index = IndexStats::new("pk", "BTree")
        .with_entry_count(100_000)
        .with_depth(4);

    let scan_cost = estimator.estimate_scan(&stats);
    let index_cost = estimator.estimate_index_lookup(&index, 0.01); // 1% selectivity

    assert!(
        index_cost.total < scan_cost.total,
        "Index lookup should be cheaper than scan"
    );
}

#[test]
fn test_vector_search_cost() {
    let estimator = CostEstimator::default();

    let cost = estimator.estimate_vector_search(10, 100, 100_000);

    assert!(cost.total > 0.0);
    assert_eq!(cost.rows, 10);
    assert!(cost.startup < cost.total);
}

#[test]
fn test_graph_traversal_cost() {
    let estimator = CostEstimator::default();

    let cost = estimator.estimate_graph_traversal(5.0, 3, 100);

    assert!(cost.total > 0.0);
    assert_eq!(cost.rows, 100);
}

#[test]
fn test_filter_reduces_rows() {
    let estimator = CostEstimator::default();

    let cost = estimator.estimate_filter(10_000, 0.1);

    assert_eq!(cost.rows, 1_000);
}

#[test]
fn test_cost_comparison() {
    let estimator = CostEstimator::default();

    let cheap = OperationCost::new(0.0, 10.0, 100);
    let expensive = OperationCost::new(0.0, 100.0, 100);

    let winner = estimator.cheaper(&cheap, &expensive);
    assert!((winner.total - 10.0).abs() < f64::EPSILON);
}

#[test]
fn test_cost_chaining() {
    let scan = OperationCost::new(0.0, 100.0, 10_000);
    let filter = OperationCost::new(0.0, 10.0, 1_000);

    let combined = scan.then(filter);

    assert!((combined.total - 110.0).abs() < f64::EPSILON);
    assert_eq!(combined.rows, 1_000);
}
