//! A static, committed model → context-window table (V2a-3 quick win).
//!
//! No notion of "which model am I compiling for, and how big is its
//! context window" exists anywhere in the crate — an agent calling
//! `compile_context` has to guess `token_budget` from scratch. This module
//! gives it a documented starting point instead: a small table, compiled
//! into the binary, dated "as of" so staleness is visible at a glance.
//! **Never a network call** — extending or refreshing the table is a code
//! change (a new release), not a runtime lookup.

use schemars::JsonSchema;
use serde::Serialize;

/// The as-of date of [`MODEL_WINDOWS`] — bump this (and add a CHANGELOG
/// entry) whenever the table gains or corrects an entry.
const MODEL_WINDOWS_AS_OF: &str = "2026-07";

/// `(model name, context window in tokens)`. Matched case-insensitively on
/// the whole string — no fuzzy or prefix matching, a caller must name their
/// model precisely (provider docs are the source of truth; this table is a
/// convenience snapshot, not an oracle). Extend as new models ship; an
/// entry that goes stale (a provider changes a window) is corrected here in
/// a normal code change, never fetched.
const MODEL_WINDOWS: &[(&str, u64)] = &[
    ("claude-opus-4-5", 200_000),
    ("claude-sonnet-4-5", 200_000),
    ("claude-haiku-4-5", 200_000),
    ("claude-sonnet-4", 200_000),
    ("claude-3-7-sonnet", 200_000),
    ("gpt-5", 400_000),
    ("gpt-5-mini", 400_000),
    ("gpt-4.1", 1_000_000),
    ("gpt-4o", 128_000),
    ("o3", 200_000),
    ("gemini-2.5-pro", 1_000_000),
    ("gemini-2.0-flash", 1_000_000),
];

/// The context window of `model`, in tokens — `None` when it is not in the
/// static table (never a guess).
#[must_use]
pub fn model_window(model: &str) -> Option<u64> {
    MODEL_WINDOWS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(model))
        .map(|(_, window)| *window)
}

/// Output of the `suggest_budget` MCP tool: a starting `token_budget` for a
/// target model, derived from the static table alone.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SuggestedBudget {
    /// The model's context window, in tokens — `None` when `target_model`
    /// is not in the static table.
    pub window: Option<u64>,
    /// `window - reserve_tokens` (saturating at 0) — `None` when `window`
    /// is `None`. Mirrors the role of
    /// [`CompilePolicy::response_reserve_tokens`](super::CompilePolicy::response_reserve_tokens)
    /// on `compile_context`'s own budget.
    pub suggested_budget: Option<u64>,
    /// Always the static table's provenance, dated — never "measured" or
    /// "fetched": this is a committed snapshot, not a live lookup.
    pub source: String,
}

/// Look up `target_model`'s window and suggest a budget that reserves
/// `reserve_tokens` for the response. Never touches the network. An unknown
/// model reports both fields `None` — an honest "I don't know", never a
/// guessed default.
#[must_use]
pub fn suggest_token_budget(target_model: &str, reserve_tokens: u64) -> SuggestedBudget {
    let window = model_window(target_model);
    SuggestedBudget {
        window,
        suggested_budget: window.map(|w| w.saturating_sub(reserve_tokens)),
        source: format!("static table as of {MODEL_WINDOWS_AS_OF}"),
    }
}

#[cfg(test)]
#[path = "model_windows_tests.rs"]
mod tests;
