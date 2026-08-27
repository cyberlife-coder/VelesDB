//! Search quality metrics for evaluating retrieval performance.
//!
//! This module provides standard information retrieval metrics:
//! - **Recall@k**: Proportion of true neighbors found in top-k results
//! - **Precision@k**: Proportion of relevant results among top-k returned
//! - **MRR (Mean Reciprocal Rank)**: Quality of ranking based on first relevant result
//! - **NDCG@k**: Normalized Discounted Cumulative Gain for ranking quality
//! - **Hit Rate**: Proportion of queries with at least one relevant result
//! - **MAP**: Mean Average Precision across multiple queries

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
    // Reason: n is the number of query-result pairs (bounded by input slice length);
    // f64 is exact for integers up to 2^53, so no precision loss in practice.
    let n_f64 = n as f64;
    (
        total_recall / n_f64,
        total_precision / n_f64,
        total_mrr / n_f64,
    )
}

/// Computes the Discounted Cumulative Gain for a relevance slice truncated to `k`.
#[allow(clippy::cast_precision_loss)]
// Reason: i is a loop index (0..k where k ≤ slice length); f64 is exact for
// integers up to 2^53, so casting a small index to f64 loses no precision.
fn compute_dcg(relevances: &[f64], k: usize) -> f64 {
    relevances
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| {
            let gain = 2.0_f64.powf(rel) - 1.0;
            let discount = (i as f64 + 2.0).log2();
            gain / discount
        })
        .sum()
}

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

    let dcg = compute_dcg(relevances, k);

    let mut sorted_relevances = relevances.to_vec();
    sorted_relevances.sort_unstable_by(|a, b| {
        b.partial_cmp(a)
            .unwrap_or_else(|| match (a.is_nan(), b.is_nan()) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Equal,
            })
    });
    let idcg = compute_dcg(&sorted_relevances, k);

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
/// MAP is the mean of Average Precision (AP) over all queries. AP rewards a
/// system that returns relevant items early *and* penalises one that misses
/// relevant items entirely — the second half is why `total_relevant` is a
/// parameter rather than something derived from the flags.
///
/// # Formula
///
/// `AP = (1/R) * Σ P(k) * rel(k)` where `R` is the total number of relevant
/// items **in the corpus**, not the number this query happened to retrieve.
/// `MAP = (1/Q) * Σ AP_q` where `Q` is the number of queries.
///
/// # Why the signature carries `total_relevant`
///
/// This used to take `&[Vec<bool>]` and divide by the count of relevant items
/// *retrieved*, while its documentation stated the formula above. The two
/// disagree exactly where the metric earns its keep: retrieving 1 of 10
/// relevant documents, at rank 1, scored `AP = 1.0` — a perfect score for 10%
/// recall. A ranking quality metric that cannot see what it missed will always
/// flatter a system that returns one confident result and stops.
///
/// The old shape could not express the fix: `&[bool]` over retrieved positions
/// simply does not know `R`. So the signature moved rather than the doc. Where
/// a caller genuinely has no corpus-wide count, passing the retrieved-relevant
/// count reproduces the previous behaviour — explicitly, at the call site,
/// instead of silently inside the metric.
///
/// # Arguments
///
/// * `queries` - one entry per query: the relevance flags at each retrieved
///   position (`true` = relevant), paired with the total number of relevant
///   items in the corpus for that query.
///
/// # Returns
///
/// A value between 0.0 and 1.0. A query whose `total_relevant` is 0 contributes
/// 0.0: there was nothing to find, so no ranking of it can be credited.
///
/// `total_relevant` below the number actually retrieved is a caller error — it
/// describes a corpus with fewer relevant items than the results contain. Rather
/// than return an `AP > 1.0` that would quietly corrupt an average, the
/// denominator is raised to the retrieved count, which keeps the result inside
/// its documented range.
#[must_use]
pub fn mean_average_precision(queries: &[(&[bool], usize)]) -> f64 {
    if queries.is_empty() {
        return 0.0;
    }

    let total_ap: f64 = queries
        .iter()
        .map(|(relevances, total_relevant)| {
            let mut retrieved_relevant = 0u32;
            let mut precision_sum = 0.0;

            for (index, &is_relevant) in relevances.iter().enumerate() {
                if is_relevant {
                    retrieved_relevant += 1;
                    #[allow(clippy::cast_precision_loss)]
                    let precision_at_index = f64::from(retrieved_relevant) / (index + 1) as f64;
                    precision_sum += precision_at_index;
                }
            }

            let denominator = (*total_relevant).max(retrieved_relevant as usize);
            if denominator == 0 {
                0.0
            } else {
                #[allow(clippy::cast_precision_loss)]
                let denominator = denominator as f64;
                precision_sum / denominator
            }
        })
        .sum();

    #[allow(clippy::cast_precision_loss)]
    let map = total_ap / queries.len() as f64;
    map
}

#[cfg(test)]
#[path = "retrieval_tests.rs"]
mod tests;
