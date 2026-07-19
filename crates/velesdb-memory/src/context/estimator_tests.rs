//! Unit tests for the char-class token estimator.

use super::*;

#[test]
fn test_estimate_empty_text_is_zero() {
    assert_eq!(HeuristicEstimator.estimate(""), 0);
    assert_eq!(
        HeuristicEstimator.estimate("   \t "),
        0,
        "spaces/tabs are free"
    );
    // Newlines are never free — even alone they cost a (rounded-up) token.
    assert_eq!(HeuristicEstimator.estimate("\n"), 1);
}

#[test]
fn test_estimate_rounds_up_per_word() {
    // 1 other-char word → ceil(9/30) = 1: even a single char costs a token.
    assert_eq!(HeuristicEstimator.estimate("x"), 1);
    // 5 other chars → ceil(45/30) = 2.
    assert_eq!(HeuristicEstimator.estimate("abcde"), 2);
    // Two short words cost one token each (per-word ceiling).
    assert_eq!(HeuristicEstimator.estimate("a b"), 2);
}

#[test]
fn test_estimate_digits_cost_a_full_token_each() {
    // "2026-07-14": 8 digits (30 each) + 2 dashes (9 each) → ceil(258/30) = 9.
    assert_eq!(HeuristicEstimator.estimate("2026-07-14"), 9);
}

#[test]
fn test_estimate_cjk_costs_nearly_a_token_per_char() {
    // 5 CJK chars in one run → ceil(125/30) = 5 (real BPE: ~1.39 chars/token,
    // so this over-counts — the safe side).
    assert_eq!(HeuristicEstimator.estimate("五五五五五"), 5);
}

#[test]
fn test_estimate_spaces_and_tabs_are_free() {
    assert_eq!(
        HeuristicEstimator.estimate("alpha beta"),
        HeuristicEstimator.estimate("alpha    \t   beta"),
        "BPE folds spaces into the following token; the estimate must too"
    );
}

#[test]
fn test_estimate_newlines_cost_half_a_token_each() {
    let flat = HeuristicEstimator.estimate("alpha beta");
    // "\n\n" is one real BPE token — the estimate charges exactly one for it.
    assert_eq!(HeuristicEstimator.estimate("alpha\n\nbeta"), flat + 1);
    // A lone newline still rounds up to one token (the safe side).
    assert_eq!(HeuristicEstimator.estimate("alpha\nbeta"), flat + 1);
}

#[test]
fn test_estimate_is_deterministic() {
    let text = "the ingestion worker retried the batch 3 times après l'échec 五";
    assert_eq!(
        HeuristicEstimator.estimate(text),
        HeuristicEstimator.estimate(text)
    );
}

#[test]
fn test_estimate_forwards_through_a_box() {
    let boxed: DynTokenEstimator = Box::new(HeuristicEstimator);
    assert_eq!(boxed.estimate("abcde"), 2);
    assert_eq!(boxed.bytes_per_token_hint(), 3);
}

#[test]
fn test_estimate_superadditive_so_piecewise_sums_bound_the_whole() {
    // Summing per-piece estimates must bound the estimate of the
    // concatenation — the packing budget guarantee rests on it. The worst
    // case for a per-word estimator is a chunk cut in the middle of a word
    // (the halves merge back into one word), including across char classes.
    for (a, b) in [
        ("abc", "defg"),
        ("abc", "123"),
        ("abc", "五五"),
        ("2026-07", "-14"),
        ("五五", "五五五"),
    ] {
        let whole = format!("{a}{b}");
        assert!(
            HeuristicEstimator.estimate(a) + HeuristicEstimator.estimate(b)
                >= HeuristicEstimator.estimate(&whole),
            "est({a:?}) + est({b:?}) must bound est({whole:?})"
        );
    }
}

#[test]
fn test_estimate_overcounts_typical_prose() {
    // Anchored expectation from the cl100k calibration: this English
    // sentence is ~13 real BPE tokens; the estimate must land above it
    // (safety) but below double (utilization).
    let text = "The deploy pipeline runs clippy before any artifact ships.";
    let estimate = HeuristicEstimator.estimate(text);
    assert!((13..=26).contains(&estimate), "estimate = {estimate}");
}
