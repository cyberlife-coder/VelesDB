//! Tests for metrics module (extracted for maintainability)

// =============================================================================
// TDD Tests - Written BEFORE implementation (following WIS-77 requirements)
// =============================================================================

use crate::metrics::*;

// =========================================================================
// Recall@k Tests
// =========================================================================

#[test]
fn test_recall_at_k_perfect() {
    // Arrange: all ground truth items are in results
    let ground_truth = vec![1u64, 2, 3, 4, 5];
    let results = vec![1u64, 2, 3, 4, 5];

    // Act
    let recall = recall_at_k(&ground_truth, &results);

    // Assert: 100% recall
    assert!(
        (recall - 1.0).abs() < f64::EPSILON,
        "Expected 1.0, got {recall}"
    );
}

#[test]
fn test_recall_at_k_partial() {
    // Arrange: 3 out of 5 ground truth items found
    let ground_truth = vec![1u64, 2, 3, 4, 5];
    let results = vec![1u64, 3, 6, 2, 7];

    // Act
    let recall = recall_at_k(&ground_truth, &results);

    // Assert: 3/5 = 0.6
    assert!(
        (recall - 0.6).abs() < f64::EPSILON,
        "Expected 0.6, got {recall}"
    );
}

#[test]
fn test_recall_at_k_zero() {
    // Arrange: no ground truth items in results
    let ground_truth = vec![1u64, 2, 3];
    let results = vec![10u64, 20, 30];

    // Act
    let recall = recall_at_k(&ground_truth, &results);

    // Assert: 0% recall
    assert!(
        (recall - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {recall}"
    );
}

#[test]
fn test_recall_at_k_empty_ground_truth() {
    // Arrange: empty ground truth
    let ground_truth: Vec<u64> = vec![];
    let results = vec![1u64, 2, 3];

    // Act
    let recall = recall_at_k(&ground_truth, &results);

    // Assert: 0.0 (edge case)
    assert!(
        (recall - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {recall}"
    );
}

#[test]
fn test_recall_at_k_empty_results() {
    // Arrange: empty results
    let ground_truth = vec![1u64, 2, 3];
    let results: Vec<u64> = vec![];

    // Act
    let recall = recall_at_k(&ground_truth, &results);

    // Assert: 0% recall
    assert!(
        (recall - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {recall}"
    );
}

// =========================================================================
// Precision@k Tests
// =========================================================================

#[test]
fn test_precision_at_k_perfect() {
    // Arrange: all results are relevant
    let ground_truth = vec![1u64, 2, 3, 4, 5];
    let results = vec![1u64, 2, 3, 4, 5];

    // Act
    let precision = precision_at_k(&ground_truth, &results);

    // Assert: 100% precision
    assert!(
        (precision - 1.0).abs() < f64::EPSILON,
        "Expected 1.0, got {precision}"
    );
}

#[test]
fn test_precision_at_k_partial() {
    // Arrange: 3 out of 5 results are relevant
    let ground_truth = vec![1u64, 2, 3, 4, 5];
    let results = vec![1u64, 3, 6, 2, 7];

    // Act
    let precision = precision_at_k(&ground_truth, &results);

    // Assert: 3/5 = 0.6
    assert!(
        (precision - 0.6).abs() < f64::EPSILON,
        "Expected 0.6, got {precision}"
    );
}

#[test]
fn test_precision_at_k_zero() {
    // Arrange: no results are relevant
    let ground_truth = vec![1u64, 2, 3];
    let results = vec![10u64, 20, 30];

    // Act
    let precision = precision_at_k(&ground_truth, &results);

    // Assert: 0% precision
    assert!(
        (precision - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {precision}"
    );
}

#[test]
fn test_precision_at_k_empty_results() {
    // Arrange: empty results
    let ground_truth = vec![1u64, 2, 3];
    let results: Vec<u64> = vec![];

    // Act
    let precision = precision_at_k(&ground_truth, &results);

    // Assert: 0.0 (edge case)
    assert!(
        (precision - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {precision}"
    );
}

#[test]
fn test_precision_different_k() {
    // Arrange: more results than ground truth
    let ground_truth = vec![1u64, 2, 3];
    let results = vec![1u64, 2, 3, 10, 20, 30, 40, 50, 60, 70]; // 10 results, 3 relevant

    // Act
    let precision = precision_at_k(&ground_truth, &results);

    // Assert: 3/10 = 0.3
    assert!(
        (precision - 0.3).abs() < f64::EPSILON,
        "Expected 0.3, got {precision}"
    );
}

// =========================================================================
// MRR Tests
// =========================================================================

#[test]
fn test_mrr_first_relevant() {
    // Arrange: first result is relevant
    let ground_truth = vec![1u64, 2, 3];
    let results = vec![1u64, 10, 20, 30];

    // Act
    let mrr_val = mrr(&ground_truth, &results);

    // Assert: 1/1 = 1.0
    assert!(
        (mrr_val - 1.0).abs() < f64::EPSILON,
        "Expected 1.0, got {mrr_val}"
    );
}

#[test]
fn test_mrr_second_relevant() {
    // Arrange: second result is relevant
    let ground_truth = vec![1u64, 2, 3];
    let results = vec![10u64, 1, 20, 30];

    // Act
    let mrr_val = mrr(&ground_truth, &results);

    // Assert: 1/2 = 0.5
    assert!(
        (mrr_val - 0.5).abs() < f64::EPSILON,
        "Expected 0.5, got {mrr_val}"
    );
}

#[test]
fn test_mrr_third_relevant() {
    // Arrange: third result is relevant
    let ground_truth = vec![1u64, 2, 3];
    let results = vec![10u64, 20, 2, 30];

    // Act
    let mrr_val = mrr(&ground_truth, &results);

    // Assert: 1/3 ≈ 0.333...
    let expected = 1.0 / 3.0;
    assert!(
        (mrr_val - expected).abs() < f64::EPSILON,
        "Expected {expected}, got {mrr_val}"
    );
}

#[test]
fn test_mrr_no_relevant() {
    // Arrange: no relevant results
    let ground_truth = vec![1u64, 2, 3];
    let results = vec![10u64, 20, 30, 40];

    // Act
    let mrr_val = mrr(&ground_truth, &results);

    // Assert: 0.0
    assert!(
        (mrr_val - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {mrr_val}"
    );
}

#[test]
fn test_mrr_empty_results() {
    // Arrange: empty results
    let ground_truth = vec![1u64, 2, 3];
    let results: Vec<u64> = vec![];

    // Act
    let mrr_val = mrr(&ground_truth, &results);

    // Assert: 0.0
    assert!(
        (mrr_val - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {mrr_val}"
    );
}

// =========================================================================
// Average Metrics Tests
// =========================================================================

#[test]
fn test_average_metrics_perfect() {
    // Arrange: perfect retrieval for all queries
    let ground_truths = vec![vec![1u64, 2, 3], vec![4u64, 5, 6]];
    let results_list = vec![vec![1u64, 2, 3], vec![4u64, 5, 6]];

    // Act
    let (avg_recall, avg_precision, avg_mrr) = average_metrics(&ground_truths, &results_list);

    // Assert: all metrics = 1.0
    assert!(
        (avg_recall - 1.0).abs() < f64::EPSILON,
        "Expected recall 1.0, got {avg_recall}"
    );
    assert!(
        (avg_precision - 1.0).abs() < f64::EPSILON,
        "Expected precision 1.0, got {avg_precision}"
    );
    assert!(
        (avg_mrr - 1.0).abs() < f64::EPSILON,
        "Expected MRR 1.0, got {avg_mrr}"
    );
}

#[test]
fn test_average_metrics_mixed() {
    // Arrange: mixed retrieval quality
    let ground_truths = vec![
        vec![1u64, 2, 3, 4, 5],      // Query 1: need 5 items
        vec![10u64, 20, 30, 40, 50], // Query 2: need 5 items
    ];
    let results_list = vec![
        vec![1u64, 2, 3, 10, 20],    // Query 1: 3/5 relevant, first is relevant
        vec![10u64, 11, 12, 13, 14], // Query 2: 1/5 relevant, first is relevant
    ];

    // Act
    let (avg_recall, avg_precision, avg_mrr) = average_metrics(&ground_truths, &results_list);

    // Assert
    // Query 1: recall=3/5=0.6, precision=3/5=0.6, mrr=1.0
    // Query 2: recall=1/5=0.2, precision=1/5=0.2, mrr=1.0
    // Avg: recall=(0.6+0.2)/2=0.4, precision=(0.6+0.2)/2=0.4, mrr=(1.0+1.0)/2=1.0
    assert!(
        (avg_recall - 0.4).abs() < f64::EPSILON,
        "Expected recall 0.4, got {avg_recall}"
    );
    assert!(
        (avg_precision - 0.4).abs() < f64::EPSILON,
        "Expected precision 0.4, got {avg_precision}"
    );
    assert!(
        (avg_mrr - 1.0).abs() < f64::EPSILON,
        "Expected MRR 1.0, got {avg_mrr}"
    );
}

#[test]
fn test_average_metrics_empty() {
    // Arrange: empty inputs
    let ground_truths: Vec<Vec<u64>> = vec![];
    let results_list: Vec<Vec<u64>> = vec![];

    // Act
    let (avg_recall, avg_precision, avg_mrr) = average_metrics(&ground_truths, &results_list);

    // Assert: all 0.0
    assert!((avg_recall - 0.0).abs() < f64::EPSILON);
    assert!((avg_precision - 0.0).abs() < f64::EPSILON);
    assert!((avg_mrr - 0.0).abs() < f64::EPSILON);
}

// =========================================================================
// Exact Search Recall Validation (WIS-77 requirement)
// =========================================================================

#[test]
fn test_exact_search_has_100_percent_recall() {
    // Arrange: simulate exact brute-force search
    // Ground truth and results should be identical for exact search
    let ground_truth: Vec<u64> = (0..100).collect();
    let exact_results: Vec<u64> = (0..100).collect();

    // Act
    let recall = recall_at_k(&ground_truth, &exact_results);
    let precision = precision_at_k(&ground_truth, &exact_results);

    // Assert: exact search = 100% recall and precision
    assert!(
        (recall - 1.0).abs() < f64::EPSILON,
        "Exact search must have 100% recall"
    );
    assert!(
        (precision - 1.0).abs() < f64::EPSILON,
        "Exact search must have 100% precision"
    );
}

#[test]
fn test_recall_at_10() {
    // Arrange: k=10 scenario
    let ground_truth: Vec<u64> = (0..10).collect();
    let results = vec![0u64, 1, 2, 3, 4, 5, 6, 7, 100, 101]; // 8 correct, 2 wrong

    // Act
    let recall = recall_at_k(&ground_truth, &results);

    // Assert: 8/10 = 0.8
    assert!(
        (recall - 0.8).abs() < f64::EPSILON,
        "Expected 0.8, got {recall}"
    );
}

#[test]
fn test_recall_at_100() {
    // Arrange: k=100 scenario
    let ground_truth: Vec<u64> = (0..100).collect();
    let mut results: Vec<u64> = (0..90).collect(); // 90 correct
    results.extend(200..210); // 10 wrong

    // Act
    let recall = recall_at_k(&ground_truth, &results);

    // Assert: 90/100 = 0.9
    assert!(
        (recall - 0.9).abs() < f64::EPSILON,
        "Expected 0.9, got {recall}"
    );
}

// =========================================================================
// WIS-86: NDCG@k Tests
// =========================================================================

#[test]
fn test_ndcg_perfect_ranking() {
    // Arrange: perfect ranking (relevance scores in descending order)
    let relevances = vec![3.0, 2.0, 1.0, 0.0];

    // Act
    let ndcg = ndcg_at_k(&relevances, 4);

    // Assert: perfect ranking = 1.0
    assert!((ndcg - 1.0).abs() < 1e-10, "Expected 1.0, got {ndcg}");
}

#[test]
fn test_ndcg_worst_ranking() {
    // Arrange: reversed ranking (worst case)
    let relevances = vec![0.0, 1.0, 2.0, 3.0];

    // Act
    let ndcg = ndcg_at_k(&relevances, 4);

    // Assert: should be < 1.0 (penalized for wrong order)
    assert!(
        ndcg < 1.0,
        "NDCG should be < 1.0 for bad ranking, got {ndcg}"
    );
    assert!(ndcg > 0.0, "NDCG should be > 0.0, got {ndcg}");
}

#[test]
fn test_ndcg_empty() {
    // Arrange: empty relevances
    let relevances: Vec<f64> = vec![];

    // Act
    let ndcg = ndcg_at_k(&relevances, 10);

    // Assert: 0.0 for empty input
    assert!(
        (ndcg - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {ndcg}"
    );
}

#[test]
fn test_ndcg_k_greater_than_list() {
    // Arrange: k > list length
    let relevances = vec![3.0, 2.0];

    // Act
    let ndcg = ndcg_at_k(&relevances, 10);

    // Assert: should still work, using available items
    assert!((ndcg - 1.0).abs() < 1e-10, "Expected 1.0, got {ndcg}");
}

#[test]
fn test_ndcg_all_zeros() {
    // Arrange: no relevant items
    let relevances = vec![0.0, 0.0, 0.0];

    // Act
    let ndcg = ndcg_at_k(&relevances, 3);

    // Assert: 0.0 when no relevant items
    assert!(
        (ndcg - 0.0).abs() < f64::EPSILON,
        "Expected 0.0, got {ndcg}"
    );
}

// =========================================================================
// WIS-86: Hit Rate Tests
// =========================================================================

#[test]
fn test_hit_rate_all_hits() {
    // Arrange: all queries have at least one relevant result
    let query_results = vec![
        (vec![1u64, 2, 3], vec![1u64, 10, 20]), // hit: 1 is relevant
        (vec![4u64, 5, 6], vec![4u64, 5, 30]),  // hit: 4, 5 are relevant
    ];

    // Act
    let hr = hit_rate(&query_results, 3);

    // Assert: 100% hit rate
    assert!((hr - 1.0).abs() < f64::EPSILON, "Expected 1.0, got {hr}");
}

#[test]
fn test_hit_rate_no_hits() {
    // Arrange: no queries have relevant results in top-k
    let query_results = vec![
        (vec![1u64, 2, 3], vec![10u64, 20, 30]), // no hit
        (vec![4u64, 5, 6], vec![40u64, 50, 60]), // no hit
    ];

    // Act
    let hr = hit_rate(&query_results, 3);

    // Assert: 0% hit rate
    assert!((hr - 0.0).abs() < f64::EPSILON, "Expected 0.0, got {hr}");
}

#[test]
fn test_hit_rate_partial() {
    // Arrange: 1 out of 2 queries has a hit
    let query_results = vec![
        (vec![1u64, 2, 3], vec![1u64, 10, 20]),  // hit
        (vec![4u64, 5, 6], vec![40u64, 50, 60]), // no hit
    ];

    // Act
    let hr = hit_rate(&query_results, 3);

    // Assert: 50% hit rate
    assert!((hr - 0.5).abs() < f64::EPSILON, "Expected 0.5, got {hr}");
}

#[test]
fn test_hit_rate_empty() {
    // Arrange: no queries
    let query_results: Vec<(Vec<u64>, Vec<u64>)> = vec![];

    // Act
    let hr = hit_rate(&query_results, 3);

    // Assert: 0.0 for empty input
    assert!((hr - 0.0).abs() < f64::EPSILON, "Expected 0.0, got {hr}");
}

// =========================================================================
// WIS-86: MAP (Mean Average Precision) Tests
// =========================================================================

#[test]
fn test_map_perfect() {
    // Arrange: all results are relevant (perfect precision at every position)
    let relevance_lists = vec![
        vec![true, true, true], // Query 1: all relevant
        vec![true, true, true], // Query 2: all relevant
    ];

    // Act
    let map = mean_average_precision(&relevance_lists);

    // Assert: 1.0 (perfect MAP)
    assert!((map - 1.0).abs() < f64::EPSILON, "Expected 1.0, got {map}");
}

#[test]
fn test_map_no_relevant() {
    // Arrange: no relevant results
    let relevance_lists = vec![vec![false, false, false], vec![false, false, false]];

    // Act
    let map = mean_average_precision(&relevance_lists);

    // Assert: 0.0
    assert!((map - 0.0).abs() < f64::EPSILON, "Expected 0.0, got {map}");
}

#[test]
fn test_map_mixed() {
    // Arrange: mixed relevance
    // Query 1: [true, false, true] -> AP = (1/1 + 2/3) / 2 = 0.833...
    // Query 2: [false, true, false] -> AP = 1/2 / 1 = 0.5
    // MAP = (0.833 + 0.5) / 2 = 0.666...
    let relevance_lists = vec![vec![true, false, true], vec![false, true, false]];

    // Act
    let map = mean_average_precision(&relevance_lists);

    // Assert: should be around 0.666
    let expected = ((1.0 + 2.0 / 3.0) / 2.0 + 0.5) / 2.0;
    assert!(
        (map - expected).abs() < 1e-10,
        "Expected {expected}, got {map}"
    );
}

#[test]
fn test_map_empty() {
    // Arrange: no queries
    let relevance_lists: Vec<Vec<bool>> = vec![];

    // Act
    let map = mean_average_precision(&relevance_lists);

    // Assert: 0.0
    assert!((map - 0.0).abs() < f64::EPSILON, "Expected 0.0, got {map}");
}

// =========================================================================
// WIS-87: Latency Percentiles Tests
// =========================================================================

#[test]
fn test_latency_stats_basic() {
    use std::time::Duration;

    // Arrange: simple latency samples
    let samples: Vec<Duration> = vec![
        Duration::from_micros(100),
        Duration::from_micros(200),
        Duration::from_micros(300),
        Duration::from_micros(400),
        Duration::from_micros(500),
    ];

    // Act
    let stats = compute_latency_percentiles(&samples);

    // Assert
    assert_eq!(stats.min, Duration::from_micros(100));
    assert_eq!(stats.max, Duration::from_micros(500));
    assert_eq!(stats.p50, Duration::from_micros(300)); // median
}

#[test]
fn test_latency_stats_single_sample() {
    use std::time::Duration;

    // Arrange: single sample
    let samples = vec![Duration::from_micros(100)];

    // Act
    let stats = compute_latency_percentiles(&samples);

    // Assert: all percentiles should be the same
    assert_eq!(stats.min, Duration::from_micros(100));
    assert_eq!(stats.max, Duration::from_micros(100));
    assert_eq!(stats.p50, Duration::from_micros(100));
    assert_eq!(stats.p95, Duration::from_micros(100));
    assert_eq!(stats.p99, Duration::from_micros(100));
}

#[test]
fn test_latency_stats_empty() {
    use std::time::Duration;

    // Arrange: no samples
    let samples: Vec<Duration> = vec![];

    // Act
    let stats = compute_latency_percentiles(&samples);

    // Assert: all zeros
    assert_eq!(stats.min, Duration::ZERO);
    assert_eq!(stats.max, Duration::ZERO);
    assert_eq!(stats.p50, Duration::ZERO);
}

#[test]
fn test_latency_stats_p99() {
    use std::time::Duration;

    // Arrange: 100 samples to test p99
    let samples: Vec<Duration> = (1..=100).map(|i| Duration::from_micros(i * 10)).collect();

    // Act
    let stats = compute_latency_percentiles(&samples);

    // Assert: p99 should be near the 99th value
    assert!(stats.p99 >= Duration::from_micros(990));
    assert!(stats.p99 <= Duration::from_micros(1000));
}

#[test]
fn test_latency_stats_mean() {
    use std::time::Duration;

    // Arrange: known mean
    let samples = vec![
        Duration::from_micros(100),
        Duration::from_micros(200),
        Duration::from_micros(300),
    ];

    // Act
    let stats = compute_latency_percentiles(&samples);

    // Assert: mean = (100 + 200 + 300) / 3 = 200
    assert_eq!(stats.mean, Duration::from_micros(200));
}
