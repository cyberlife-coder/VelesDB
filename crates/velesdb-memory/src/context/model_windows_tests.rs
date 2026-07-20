//! Unit tests for the static model→window table and `suggest_token_budget`
//! (V2a-3 quick win).

use super::*;

#[test]
fn test_model_window_known_model_returns_its_window() {
    // A model that ships in the static table must resolve to a window.
    assert_eq!(model_window("claude-sonnet-4-5"), Some(200_000));
}

#[test]
fn test_model_window_is_case_insensitive() {
    assert_eq!(
        model_window("Claude-Sonnet-4-5"),
        model_window("claude-sonnet-4-5")
    );
}

#[test]
fn test_model_window_unknown_model_returns_none() {
    // Never a guess: an unrecognized model name comes back empty, not a
    // fallback default.
    assert_eq!(model_window("some-model-that-does-not-exist-2099"), None);
}

#[test]
fn test_suggest_token_budget_known_model_subtracts_reserve() {
    let suggestion = suggest_token_budget("claude-sonnet-4-5", 10_000);
    assert_eq!(suggestion.window, Some(200_000));
    assert_eq!(suggestion.suggested_budget, Some(190_000));
    assert!(suggestion.source.contains("static table"));
}

#[test]
fn test_suggest_token_budget_zero_reserve_by_default() {
    let suggestion = suggest_token_budget("claude-sonnet-4-5", 0);
    assert_eq!(suggestion.suggested_budget, suggestion.window);
}

#[test]
fn test_suggest_token_budget_reserve_saturates_at_zero() {
    // A reserve larger than the window must never underflow.
    let suggestion = suggest_token_budget("claude-sonnet-4-5", 10_000_000);
    assert_eq!(suggestion.suggested_budget, Some(0));
}

#[test]
fn test_suggest_token_budget_unknown_model_returns_nulls_not_a_guess() {
    let suggestion = suggest_token_budget("some-model-that-does-not-exist-2099", 0);
    assert_eq!(suggestion.window, None);
    assert_eq!(suggestion.suggested_budget, None);
    // The source is still reported — it explains WHY there's no number.
    assert!(suggestion.source.contains("static table"));
}
