//! WASM bindings for `velesdb-core`'s IR metrics.
//!
//! Exposes search quality evaluation functions: recall, precision, nDCG, MRR.
//! Allows agents in the browser to evaluate retrieval quality locally.

use wasm_bindgen::prelude::*;

use velesdb_core::metrics;

/// Compute Recall@k: proportion of relevant items retrieved.
///
/// `ground_truth` and `results` are arrays of item IDs (u64).
/// Returns a value between 0.0 and 1.0.
#[wasm_bindgen]
pub fn recall_at_k(ground_truth: &[u64], results: &[u64]) -> f64 {
    metrics::recall_at_k(ground_truth, results)
}

/// Compute Precision@k: proportion of retrieved items that are relevant.
///
/// Returns a value between 0.0 and 1.0.
#[wasm_bindgen]
pub fn precision_at_k(ground_truth: &[u64], results: &[u64]) -> f64 {
    metrics::precision_at_k(ground_truth, results)
}

/// Compute MRR: Mean Reciprocal Rank.
///
/// Returns `1/rank` of the first relevant result. 1.0 if first result is relevant.
#[wasm_bindgen]
pub fn mrr(ground_truth: &[u64], results: &[u64]) -> f64 {
    metrics::mrr(ground_truth, results)
}

/// Compute nDCG@k: normalized discounted cumulative gain.
///
/// `relevances` is an array of relevance scores (f64) for each result position.
/// `k` is the number of top positions to consider.
/// Returns a value between 0.0 and 1.0.
#[wasm_bindgen]
pub fn ndcg_at_k(relevances: &[f64], k: usize) -> f64 {
    metrics::ndcg_at_k(relevances, k)
}

/// Compute Hit Rate for a single query: 1.0 if at least one relevant result in top-k.
///
/// `ground_truth` and `results` are arrays of item IDs (u64).
/// `k` is the number of top positions to consider.
#[wasm_bindgen]
pub fn hit_rate_single(ground_truth: &[u64], results: &[u64], k: usize) -> f64 {
    let query_results = vec![(ground_truth.to_vec(), results.to_vec())];
    metrics::hit_rate(&query_results, k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recall_at_k_perfect() {
        let truth = [1_u64, 2, 3, 4, 5];
        let results = [1_u64, 2, 3, 4, 5];
        assert!((recall_at_k(&truth, &results) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_recall_at_k_partial() {
        let truth = [1_u64, 2, 3, 4, 5];
        let results = [1_u64, 3, 6, 2, 7];
        let r = recall_at_k(&truth, &results);
        assert!((r - 0.6).abs() < f64::EPSILON); // 3/5
    }

    #[test]
    fn test_precision_at_k_partial() {
        let truth = [1_u64, 2, 3];
        let results = [1_u64, 4, 2, 5, 6];
        let p = precision_at_k(&truth, &results);
        assert!((p - 0.4).abs() < f64::EPSILON); // 2/5
    }

    #[test]
    fn test_mrr_first_relevant() {
        let truth = [1_u64, 2, 3];
        let results = [1_u64, 4, 5];
        assert!((mrr(&truth, &results) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mrr_second_relevant() {
        let truth = [2_u64, 3];
        let results = [1_u64, 2, 4];
        assert!((mrr(&truth, &results) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ndcg_perfect_ranking() {
        // All relevances = 1.0, perfect ordering
        let relevances = [1.0, 1.0, 1.0];
        let n = ndcg_at_k(&relevances, 3);
        assert!((n - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_hit_rate_single_hit() {
        let truth = [1_u64, 2];
        let results = [3_u64, 4, 1];
        assert!((hit_rate_single(&truth, &results, 3) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hit_rate_single_miss() {
        let truth = [1_u64, 2];
        let results = [3_u64, 4, 5];
        assert!(hit_rate_single(&truth, &results, 3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_empty_inputs() {
        assert!(recall_at_k(&[], &[1_u64, 2]).abs() < f64::EPSILON);
        assert!(precision_at_k(&[1_u64], &[]).abs() < f64::EPSILON);
        assert!(mrr(&[], &[1_u64]).abs() < f64::EPSILON);
    }
}
