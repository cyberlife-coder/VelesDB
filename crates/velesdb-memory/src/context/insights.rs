//! Savings accounting: tokens (always) and money (only when a pricing table
//! is injected).
//!
//! Money is integer micro-units of one currency (1 unit = 10⁻⁶ of the
//! currency's major unit, e.g. `1_000_000` micros = 1 EUR) — no floats, no
//! rounding drift. Prices are **injected and versioned**, never hardcoded:
//! the compiler works fully without a pricing table, reporting tokens only.
//!
//! The token figures are *local estimates* (see
//! [`super::estimator::HeuristicEstimator`]), not the provider's exact count,
//! nor billed tokens, nor cache-read tokens — four different numbers a caller
//! must not conflate. The insights report says which one it carries.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Pricing of one model, in integer micro-units per **million** input tokens
/// (e.g. a `3 USD / 1M tokens` rate is `3_000_000`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ModelPricing {
    /// Micro-units of the table's currency per million input tokens.
    pub input_micros_per_million_tokens: u64,
}

/// A versioned, caller-injected pricing table.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PricingTable {
    /// Caller-side version tag of this table (recorded in insights so a cost
    /// figure is always traceable to the prices that produced it).
    pub version: String,
    /// ISO-4217 currency code of every rate in the table (e.g. `"EUR"`).
    pub currency: String,
    /// Rates keyed by model name (a `BTreeMap` so iteration — and therefore
    /// every serialized output — is deterministically ordered).
    pub models: BTreeMap<String, ModelPricing>,
}

impl PricingTable {
    /// The cost of `tokens` input tokens on `model`, in micro-units of
    /// [`Self::currency`] — `None` when the model has no rate in the table.
    ///
    /// Saturates instead of overflowing: with realistic rates (≤ 10⁹ micros
    /// per million tokens) saturation is unreachable, and a saturated figure
    /// is still the safer answer than a wrapped one.
    #[must_use]
    pub fn cost_micros(&self, model: &str, tokens: u64) -> Option<u64> {
        let rate = self.models.get(model)?.input_micros_per_million_tokens;
        Some(tokens.saturating_mul(rate) / 1_000_000)
    }
}

/// Token and cost savings of one compilation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct CompilationInsights {
    /// Estimated tokens of all input fragments combined.
    pub tokens_in: u64,
    /// Estimated tokens of the assembled output.
    pub tokens_out: u64,
    /// `tokens_in − tokens_out` (saturating) — the local *estimate* of what
    /// this compilation avoided sending; not billed tokens.
    pub tokens_saved: u64,
    /// Tokens saved attributed to the rule that saved them, keyed by rule id
    /// (`BTreeMap` for deterministic serialization order).
    pub tokens_saved_by_rule: BTreeMap<String, u64>,
    /// Estimated cost avoided, in micro-units of [`Self::currency`] — only
    /// when a pricing table was injected *and* it prices the target model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_saved_micros: Option<u64>,
    /// Currency of the cost figure, from the pricing table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Version tag of the pricing table that produced the cost figure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<String>,
}

#[cfg(test)]
#[path = "insights_tests.rs"]
mod tests;
