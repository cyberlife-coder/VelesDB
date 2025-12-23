//! Search quality metrics for evaluating retrieval performance.
//!
//! This module provides standard information retrieval metrics:
//! - **Recall@k**: Proportion of true neighbors found in top-k results
//! - **Precision@k**: Proportion of relevant results among top-k returned
//! - **MRR (Mean Reciprocal Rank)**: Quality of ranking based on first relevant result
//!
//! # Example
//!
//! ```rust
//! use velesdb_core::metrics::{recall_at_k, precision_at_k, mrr};
//!
//! let ground_truth = vec![1, 2, 3, 4, 5];  // True top-5 neighbors
//! let results = vec![1, 3, 6, 2, 7];       // Retrieved results
//!
//! let recall = recall_at_k(&ground_truth, &results);      // 3/5 = 0.6
//! let precision = precision_at_k(&ground_truth, &results); // 3/5 = 0.6
//! let rank_quality = mrr(&ground_truth, &results);         // 1/1 = 1.0 (first result is relevant)
//! ```

use std::collections::HashSet;
use std::hash::Hash;

/// Calculates Recall@k: the proportion of true neighbors found in the results.
///
/// Recall measures how many of the true relevant items were retrieved.
/// A recall of 1.0 means all true neighbors were found.
///
/// # Formula
///
/// `recall@k = |ground_truth ∩ results| / |ground_truth|`
///
/// # Arguments
///
/// * `ground_truth` - The true k-nearest neighbors (expected results)
/// * `results` - The retrieved results from the search
///
/// # Returns
///
/// A value between 0.0 and 1.0, where 1.0 means perfect recall.
///
/// # Panics
///
/// Returns 0.0 if `ground_truth` is empty (to avoid division by zero).
#[must_use]
pub fn recall_at_k<T: Eq + Hash + Copy>(ground_truth: &[T], results: &[T]) -> f64 {
    if ground_truth.is_empty() {
        return 0.0;
    }

    let truth_set: HashSet<T> = ground_truth.iter().copied().collect();
    let found = results.iter().filter(|id| truth_set.contains(id)).count();

    #[allow(clippy::cast_precision_loss)]
    let recall = found as f64 / ground_truth.len() as f64;
    recall
}

/// Calculates Precision@k: the proportion of relevant results among those returned.
///
/// Precision measures how many of the retrieved items are actually relevant.
/// A precision of 1.0 means all returned results are relevant.
///
/// # Formula
///
/// `precision@k = |ground_truth ∩ results| / |results|`
///
/// # Arguments
///
/// * `ground_truth` - The true k-nearest neighbors (relevant items)
/// * `results` - The retrieved results from the search
///
/// # Returns
///
/// A value between 0.0 and 1.0, where 1.0 means perfect precision.
///
/// # Panics
///
/// Returns 0.0 if results is empty (to avoid division by zero).
#[must_use]
pub fn precision_at_k<T: Eq + Hash + Copy>(ground_truth: &[T], results: &[T]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }

    let truth_set: HashSet<T> = ground_truth.iter().copied().collect();
    let relevant = results.iter().filter(|id| truth_set.contains(id)).count();

    #[allow(clippy::cast_precision_loss)]
    let precision = relevant as f64 / results.len() as f64;
    precision
}

/// Calculates Mean Reciprocal Rank (MRR): quality based on the rank of the first relevant result.
///
/// MRR rewards systems that place a relevant result at the top of the list.
/// An MRR of 1.0 means the first result is always relevant.
///
/// # Formula
///
/// `MRR = 1 / rank_of_first_relevant_result`
///
/// # Arguments
///
/// * `ground_truth` - The set of relevant items
/// * `results` - The ranked list of retrieved results
///
/// # Returns
///
/// A value between 0.0 and 1.0, where 1.0 means the first result is relevant.
/// Returns 0.0 if no relevant result is found.
#[must_use]
pub fn mrr<T: Eq + Hash + Copy>(ground_truth: &[T], results: &[T]) -> f64 {
    let truth_set: HashSet<T> = ground_truth.iter().copied().collect();

    for (rank, id) in results.iter().enumerate() {
        if truth_set.contains(id) {
            #[allow(clippy::cast_precision_loss)]
            return 1.0 / (rank + 1) as f64;
        }
    }

    0.0
}

/// Calculates average metrics over multiple queries.
///
/// # Arguments
///
/// * `ground_truths` - List of ground truth results for each query
/// * `results_list` - List of retrieved results for each query
///
/// # Returns
///
/// A tuple of (`avg_recall`, `avg_precision`, `avg_mrr`).
#[must_use]
pub fn average_metrics<T: Eq + Hash + Copy>(
    ground_truths: &[Vec<T>],
    results_list: &[Vec<T>],
) -> (f64, f64, f64) {
    if ground_truths.is_empty() || results_list.is_empty() {
        return (0.0, 0.0, 0.0);
    }

    let n = ground_truths.len().min(results_list.len());
    let mut total_recall = 0.0;
    let mut total_precision = 0.0;
    let mut total_mrr = 0.0;

    for (gt, res) in ground_truths.iter().zip(results_list.iter()).take(n) {
        total_recall += recall_at_k(gt, res);
        total_precision += precision_at_k(gt, res);
        total_mrr += mrr(gt, res);
    }

    #[allow(clippy::cast_precision_loss)]
    let n_f64 = n as f64;
    (
        total_recall / n_f64,
        total_precision / n_f64,
        total_mrr / n_f64,
    )
}

// =============================================================================
// WIS-86: Advanced Metrics - NDCG, Hit Rate, MAP
// =============================================================================

/// Calculates NDCG@k (Normalized Discounted Cumulative Gain).
///
/// NDCG measures ranking quality by penalizing relevant items appearing
/// lower in the result list. A score of 1.0 means perfect ranking.
///
/// # Formula
///
/// `DCG@k = Σ (2^rel_i - 1) / log2(i + 2)` for i in 0..k
/// `NDCG@k = DCG@k / IDCG@k` where IDCG is DCG of ideal ranking
///
/// # Arguments
///
/// * `relevances` - Relevance scores for each result position (higher = more relevant)
/// * `k` - Number of top positions to consider
///
/// # Returns
///
/// A value between 0.0 and 1.0, where 1.0 means perfect ranking.
#[must_use]
pub fn ndcg_at_k(relevances: &[f64], k: usize) -> f64 {
    if relevances.is_empty() {
        return 0.0;
    }

    let k = k.min(relevances.len());

    // Calculate DCG (Discounted Cumulative Gain)
    let dcg: f64 = relevances
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| {
            let gain = 2.0_f64.powf(rel) - 1.0;
            #[allow(clippy::cast_precision_loss)]
            let discount = (i as f64 + 2.0).log2();
            gain / discount
        })
        .sum();

    // Calculate IDCG (Ideal DCG) - DCG with perfect ranking
    let mut sorted_relevances = relevances.to_vec();
    sorted_relevances.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let idcg: f64 = sorted_relevances
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| {
            let gain = 2.0_f64.powf(rel) - 1.0;
            #[allow(clippy::cast_precision_loss)]
            let discount = (i as f64 + 2.0).log2();
            gain / discount
        })
        .sum();

    if idcg == 0.0 {
        return 0.0;
    }

    dcg / idcg
}

/// Calculates Hit Rate (HR@k): proportion of queries with at least one relevant result.
///
/// Hit Rate is useful for recommendation systems where finding any relevant
/// item is considered a success.
///
/// # Arguments
///
/// * `query_results` - List of (`ground_truth`, `results`) pairs for each query
/// * `k` - Number of top positions to consider
///
/// # Returns
///
/// A value between 0.0 and 1.0, where 1.0 means every query had a hit.
#[must_use]
pub fn hit_rate<T: Eq + Hash + Copy>(query_results: &[(Vec<T>, Vec<T>)], k: usize) -> f64 {
    if query_results.is_empty() {
        return 0.0;
    }

    let hits = query_results
        .iter()
        .filter(|(ground_truth, results)| {
            let truth_set: HashSet<T> = ground_truth.iter().copied().collect();
            results.iter().take(k).any(|r| truth_set.contains(r))
        })
        .count();

    #[allow(clippy::cast_precision_loss)]
    let hr = hits as f64 / query_results.len() as f64;
    hr
}

/// Calculates Mean Average Precision (MAP).
///
/// MAP is the mean of Average Precision (AP) over all queries.
/// AP rewards systems that return relevant items early in the result list.
///
/// # Formula
///
/// `AP = (1/R) * Σ P(k) * rel(k)` where R is total relevant items
/// `MAP = (1/Q) * Σ AP_q` where Q is number of queries
///
/// # Arguments
///
/// * `relevance_lists` - For each query, a list of booleans indicating relevance
///   at each position (true = relevant, false = not relevant)
///
/// # Returns
///
/// A value between 0.0 and 1.0, where 1.0 means perfect precision at every position.
#[must_use]
pub fn mean_average_precision(relevance_lists: &[Vec<bool>]) -> f64 {
    if relevance_lists.is_empty() {
        return 0.0;
    }

    let total_ap: f64 = relevance_lists
        .iter()
        .map(|relevances| {
            let mut relevant_count = 0;
            let mut precision_sum = 0.0;

            for (i, &is_relevant) in relevances.iter().enumerate() {
                if is_relevant {
                    relevant_count += 1;
                    #[allow(clippy::cast_precision_loss)]
                    let precision_at_i = f64::from(relevant_count) / (i + 1) as f64;
                    precision_sum += precision_at_i;
                }
            }

            if relevant_count == 0 {
                0.0
            } else {
                precision_sum / f64::from(relevant_count)
            }
        })
        .sum();

    #[allow(clippy::cast_precision_loss)]
    let map = total_ap / relevance_lists.len() as f64;
    map
}

// =============================================================================
// WIS-87: Latency Percentiles
// =============================================================================

use std::time::Duration;

/// Statistics for latency measurements including percentiles.
///
/// Percentiles are more useful than mean for understanding real-world
/// performance, especially p99 which shows worst-case latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyStats {
    /// Minimum latency observed
    pub min: Duration,
    /// Maximum latency observed
    pub max: Duration,
    /// Mean (average) latency
    pub mean: Duration,
    /// 50th percentile (median)
    pub p50: Duration,
    /// 95th percentile
    pub p95: Duration,
    /// 99th percentile
    pub p99: Duration,
}

impl Default for LatencyStats {
    fn default() -> Self {
        Self {
            min: Duration::ZERO,
            max: Duration::ZERO,
            mean: Duration::ZERO,
            p50: Duration::ZERO,
            p95: Duration::ZERO,
            p99: Duration::ZERO,
        }
    }
}

/// Computes latency percentiles from a list of duration samples.
///
/// # Arguments
///
/// * `samples` - List of latency measurements
///
/// # Returns
///
/// A `LatencyStats` struct with min, max, mean, p50, p95, and p99.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
/// use velesdb_core::metrics::compute_latency_percentiles;
///
/// let samples: Vec<Duration> = (1..=100)
///     .map(|i| Duration::from_micros(i * 10))
///     .collect();
///
/// let stats = compute_latency_percentiles(&samples);
/// println!("p50: {:?}, p99: {:?}", stats.p50, stats.p99);
/// ```
#[must_use]
pub fn compute_latency_percentiles(samples: &[Duration]) -> LatencyStats {
    if samples.is_empty() {
        return LatencyStats::default();
    }

    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort();

    let n = sorted.len();
    let sum: Duration = sorted.iter().sum();

    #[allow(clippy::cast_possible_truncation)]
    let mean = if n > 0 {
        Duration::from_nanos((sum.as_nanos() / n as u128) as u64)
    } else {
        Duration::ZERO
    };

    LatencyStats {
        min: sorted[0],
        max: sorted[n - 1],
        mean,
        p50: percentile(&sorted, 50),
        p95: percentile(&sorted, 95),
        p99: percentile(&sorted, 99),
    }
}

/// Computes a percentile from a sorted list of durations.
fn percentile(sorted: &[Duration], p: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }

    let n = sorted.len();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let idx = ((p as f64 / 100.0) * (n - 1) as f64).round() as usize;
    sorted[idx.min(n - 1)]
}

// =============================================================================
// TDD Tests - Written BEFORE implementation (following WIS-77 requirements)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
}
