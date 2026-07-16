//! Unit tests for the heuristic token estimator.

use super::*;

#[test]
fn test_estimate_empty_text_is_zero() {
    assert_eq!(HeuristicEstimator.estimate(""), 0);
}

#[test]
fn test_estimate_rounds_up() {
    // 1 char → 2/5 → ceil = 1: even a single char costs one token.
    assert_eq!(HeuristicEstimator.estimate("x"), 1);
    // 5 chars → 10/5 = 2 exactly.
    assert_eq!(HeuristicEstimator.estimate("abcde"), 2);
    // 6 chars → 12/5 → ceil = 3.
    assert_eq!(HeuristicEstimator.estimate("abcdef"), 3);
}

#[test]
fn test_estimate_counts_chars_not_bytes() {
    // 5 chars, 15 bytes: the ratio must be char-based.
    let cjk = "五五五五五";
    assert_eq!(cjk.len(), 15);
    assert_eq!(HeuristicEstimator.estimate(cjk), 2);
}

#[test]
fn test_estimate_is_deterministic() {
    let text = "the ingestion worker retried the batch";
    assert_eq!(
        HeuristicEstimator.estimate(text),
        HeuristicEstimator.estimate(text)
    );
}

#[test]
fn test_estimate_forwards_through_a_box() {
    let boxed: DynTokenEstimator = Box::new(HeuristicEstimator);
    assert_eq!(boxed.estimate("abcde"), 2);
}

#[test]
fn test_estimate_superadditive_so_piecewise_sums_bound_the_whole() {
    // ceil(a) + ceil(b) ≥ ceil(a+b): summing per-piece estimates during
    // packing over-approximates the estimate of the assembled text, which is
    // what makes the budget guarantee hold.
    let (a, b) = ("abc", "defg");
    let whole = format!("{a}{b}");
    assert!(
        HeuristicEstimator.estimate(a) + HeuristicEstimator.estimate(b)
            >= HeuristicEstimator.estimate(&whole)
    );
}
