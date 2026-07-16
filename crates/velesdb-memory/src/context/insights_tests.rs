//! Unit tests for the savings accounting.

use std::collections::BTreeMap;

use super::*;

fn table(rate: u64) -> PricingTable {
    let mut models = BTreeMap::new();
    models.insert(
        "claude-fable-5".to_owned(),
        ModelPricing {
            input_micros_per_million_tokens: rate,
        },
    );
    PricingTable {
        version: "2026-07".to_owned(),
        currency: "EUR".to_owned(),
        models,
    }
}

#[test]
fn test_cost_micros_scales_rate_by_tokens() {
    // 3 EUR / 1M tokens, 500k tokens → 1.5 EUR = 1_500_000 micros.
    let pricing = table(3_000_000);
    assert_eq!(
        pricing.cost_micros("claude-fable-5", 500_000),
        Some(1_500_000)
    );
}

#[test]
fn test_cost_micros_unknown_model_is_none() {
    let pricing = table(3_000_000);
    assert_eq!(pricing.cost_micros("unknown-model", 1_000), None);
}

#[test]
fn test_cost_micros_zero_tokens_is_zero() {
    let pricing = table(3_000_000);
    assert_eq!(pricing.cost_micros("claude-fable-5", 0), Some(0));
}

#[test]
fn test_cost_micros_saturates_instead_of_overflowing() {
    let pricing = table(u64::MAX);
    let cost = pricing
        .cost_micros("claude-fable-5", u64::MAX)
        .expect("model is priced");
    assert_eq!(cost, u64::MAX / 1_000_000);
}

#[test]
fn test_insights_default_reports_no_cost() {
    let insights = CompilationInsights::default();
    assert!(insights.estimated_cost_saved_micros.is_none());
    assert!(insights.currency.is_none());
    assert!(insights.pricing_version.is_none());
}
