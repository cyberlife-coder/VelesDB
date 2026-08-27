//! `/search/multi` refuses fusion weights the strategy cannot honour.
//!
//! The weighted arms of this parser used to build their `FusionStrategy` as
//! struct literals, which accept any `f32` the JSON body carries. The core
//! constructors — the ones that check the weights are non-negative, finite,
//! and sum to 1.0 — were never called, so a request could name weights the
//! fusion kernel then applied verbatim, returning a 200 with a ranking nobody
//! asked for (#2095).

use super::multi::build_fusion_strategy;
use crate::types::MultiQuerySearchRequest;

/// A request with the canonical defaults, ready for one field to be perturbed.
///
/// Built by deserializing rather than by struct literal so the `#[serde]`
/// defaults are the ones under test — a literal would hardcode a second copy
/// of them here.
fn request(strategy: &str) -> MultiQuerySearchRequest {
    serde_json::from_value(serde_json::json!({
        "vectors": [[0.1f32, 0.2f32]],
        "strategy": strategy,
    }))
    .expect("test: the default multi-query body must deserialize")
}

#[test]
fn the_defaults_of_every_strategy_are_accepted() {
    for strategy in [
        "average",
        "avg",
        "maximum",
        "max",
        "rrf",
        "weighted",
        "relative_score",
        "rsf",
    ] {
        build_fusion_strategy(&request(strategy))
            .unwrap_or_else(|error| panic!("'{strategy}' must build with its defaults: {error}"));
    }
}

#[test]
fn weighted_defaults_are_the_canonical_constants() {
    let strategy = build_fusion_strategy(&request("weighted")).expect("weighted must build");
    match strategy {
        velesdb_core::FusionStrategy::Weighted {
            avg_weight,
            max_weight,
            hit_weight,
        } => {
            assert!((avg_weight - velesdb_core::DEFAULT_WEIGHTED_AVG_WEIGHT).abs() < f32::EPSILON);
            assert!((max_weight - velesdb_core::DEFAULT_WEIGHTED_MAX_WEIGHT).abs() < f32::EPSILON);
            assert!((hit_weight - velesdb_core::DEFAULT_WEIGHTED_HIT_WEIGHT).abs() < f32::EPSILON);
        }
        other => panic!("expected Weighted, got {other:?}"),
    }
}

#[test]
fn a_negative_weighted_component_is_refused() {
    let mut req = request("weighted");
    req.avg_weight = -0.2;
    req.max_weight = 0.9;
    req.hit_weight = 0.3;
    let error = build_fusion_strategy(&req).expect_err("a negative weight must be refused");
    assert!(
        error.contains("non-negative"),
        "message should name the rule it broke, got: {error}"
    );
}

#[test]
fn weighted_components_that_do_not_sum_to_one_are_refused() {
    let mut req = request("weighted");
    req.avg_weight = 0.9;
    req.max_weight = 0.9;
    req.hit_weight = 0.9;
    let error = build_fusion_strategy(&req).expect_err("weights summing to 2.7 must be refused");
    assert!(
        error.contains("sum to 1.0"),
        "message should name the rule it broke, got: {error}"
    );
}

#[test]
fn a_non_finite_weighted_component_is_refused() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut req = request("weighted");
        req.avg_weight = bad;
        let error = build_fusion_strategy(&req)
            .err()
            .unwrap_or_else(|| panic!("{bad} must be refused"));
        assert!(
            error.contains("finite") || error.contains("sum to 1.0"),
            "message should name the rule {bad} broke, got: {error}"
        );
    }
}

#[test]
fn relative_score_weights_that_do_not_sum_to_one_are_refused() {
    let mut req = request("rsf");
    req.dense_weight = 0.9;
    req.sparse_weight = 0.9;
    let error = build_fusion_strategy(&req).expect_err("weights summing to 1.8 must be refused");
    assert!(
        error.contains("sum to 1.0"),
        "message should name the rule it broke, got: {error}"
    );
}

#[test]
fn a_non_finite_relative_score_weight_is_refused() {
    let mut req = request("rsf");
    req.dense_weight = f32::NAN;
    req.sparse_weight = 0.5;
    let error = build_fusion_strategy(&req).expect_err("a NaN weight must be refused");
    assert!(
        error.contains("finite"),
        "message should name the rule it broke, got: {error}"
    );
}

#[test]
fn an_unknown_strategy_keeps_its_original_casing_in_the_message() {
    let error = build_fusion_strategy(&request("NoSuchStrategy"))
        .expect_err("an unknown strategy must be refused");
    assert!(
        error.contains("NoSuchStrategy"),
        "message should echo what the caller sent, got: {error}"
    );
}
