use super::*;

#[test]
fn test_recall_at_k_perfect() {
    let ground_truth = vec![1, 2, 3, 4, 5];
    let results = vec![1, 2, 3, 4, 5];
    let recall = recall_at_k(&ground_truth, &results);
    assert!((recall - 1.0).abs() < 1e-5);
}

#[test]
fn test_recall_at_k_partial() {
    let ground_truth = vec![1, 2, 3, 4, 5];
    let results = vec![1, 3, 6, 2, 7];
    let recall = recall_at_k(&ground_truth, &results);
    assert!((recall - 0.6).abs() < 1e-5); // 3/5
}

#[test]
fn test_recall_at_k_empty_truth() {
    let ground_truth: Vec<u64> = vec![];
    let results = vec![1, 2, 3];
    let recall = recall_at_k(&ground_truth, &results);
    assert!((recall - 0.0).abs() < 1e-5);
}

#[test]
fn test_precision_at_k_perfect() {
    let ground_truth = vec![1, 2, 3, 4, 5];
    let results = vec![1, 2, 3];
    let precision = precision_at_k(&ground_truth, &results);
    assert!((precision - 1.0).abs() < 1e-5);
}

#[test]
fn test_precision_at_k_partial() {
    let ground_truth = vec![1, 2, 3];
    let results = vec![1, 4, 5, 6, 7];
    let precision = precision_at_k(&ground_truth, &results);
    assert!((precision - 0.2).abs() < 1e-5); // 1/5
}

#[test]
fn test_precision_at_k_empty_results() {
    let ground_truth = vec![1, 2, 3];
    let results: Vec<u64> = vec![];
    let precision = precision_at_k(&ground_truth, &results);
    assert!((precision - 0.0).abs() < 1e-5);
}

#[test]
fn test_mrr_first_relevant() {
    let ground_truth = vec![1, 2, 3];
    let results = vec![1, 4, 5];
    let rank = mrr(&ground_truth, &results);
    assert!((rank - 1.0).abs() < 1e-5); // First result is relevant
}

#[test]
fn test_mrr_second_relevant() {
    let ground_truth = vec![1, 2, 3];
    let results = vec![4, 1, 5];
    let rank = mrr(&ground_truth, &results);
    assert!((rank - 0.5).abs() < 1e-5); // 1/2
}

#[test]
fn test_mrr_no_relevant() {
    let ground_truth = vec![1, 2, 3];
    let results = vec![4, 5, 6];
    let rank = mrr(&ground_truth, &results);
    assert!((rank - 0.0).abs() < 1e-5);
}
