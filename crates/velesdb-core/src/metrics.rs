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
