use super::parse_fusion_strategy;
use crate::types::FusionRequest;

fn strat(name: &str) -> FusionRequest {
    FusionRequest {
        strategy: name.to_string(),
        k: None,
        dense_w: None,
        sparse_w: None,
        avg_w: None,
        max_w: None,
        hit_w: None,
    }
}

#[test]
fn test_default_is_rrf_k60() {
    let result = parse_fusion_strategy(None).expect("None must default to RRF");
    match result {
        velesdb_core::FusionStrategy::RRF { k } => assert_eq!(k, 60),
        other => panic!("expected RRF, got {other:?}"),
    }
}

#[test]
fn test_rrf_with_custom_k() {
    let mut req = strat("rrf");
    req.k = Some(120);
    let result = parse_fusion_strategy(Some(&req)).expect("rrf must parse");
    match result {
        velesdb_core::FusionStrategy::RRF { k } => assert_eq!(k, 120),
        other => panic!("expected RRF, got {other:?}"),
    }
}

#[test]
fn test_average_no_params() {
    for alias in ["average", "avg", "AVG"] {
        let req = strat(alias);
        let result =
            parse_fusion_strategy(Some(&req)).unwrap_or_else(|_| panic!("'{alias}' must parse"));
        assert!(matches!(result, velesdb_core::FusionStrategy::Average));
    }
}

#[test]
fn test_maximum_no_params() {
    for alias in ["maximum", "max", "MAX"] {
        let req = strat(alias);
        let result =
            parse_fusion_strategy(Some(&req)).unwrap_or_else(|_| panic!("'{alias}' must parse"));
        assert!(matches!(result, velesdb_core::FusionStrategy::Maximum));
    }
}

#[test]
fn test_weighted_with_defaults() {
    let req = strat("weighted");
    let result = parse_fusion_strategy(Some(&req)).expect("weighted must parse");
    match result {
        velesdb_core::FusionStrategy::Weighted {
            avg_weight,
            max_weight,
            hit_weight,
        } => {
            assert!((avg_weight - 0.5).abs() < f32::EPSILON);
            assert!((max_weight - 0.3).abs() < f32::EPSILON);
            assert!((hit_weight - 0.2).abs() < f32::EPSILON);
        }
        other => panic!("expected Weighted, got {other:?}"),
    }
}

#[test]
fn test_weighted_with_explicit_weights() {
    let mut req = strat("weighted");
    req.avg_w = Some(0.7);
    req.max_w = Some(0.2);
    req.hit_w = Some(0.1);
    let result = parse_fusion_strategy(Some(&req)).expect("weighted must parse");
    match result {
        velesdb_core::FusionStrategy::Weighted {
            avg_weight,
            max_weight,
            hit_weight,
        } => {
            assert!((avg_weight - 0.7).abs() < f32::EPSILON);
            assert!((max_weight - 0.2).abs() < f32::EPSILON);
            assert!((hit_weight - 0.1).abs() < f32::EPSILON);
        }
        other => panic!("expected Weighted, got {other:?}"),
    }
}

#[test]
fn test_rsf_with_dense_weight_only() {
    let mut req = strat("rsf");
    req.dense_w = Some(0.7);
    let result = parse_fusion_strategy(Some(&req)).expect("rsf must parse");
    match result {
        velesdb_core::FusionStrategy::RelativeScore {
            dense_weight,
            sparse_weight,
        } => {
            assert!((dense_weight - 0.7).abs() < f32::EPSILON);
            assert!((sparse_weight - 0.3).abs() < f32::EPSILON);
        }
        other => panic!("expected RelativeScore, got {other:?}"),
    }
}

#[test]
fn test_relative_score_alias() {
    let req = strat("relative_score");
    let result = parse_fusion_strategy(Some(&req)).expect("relative_score must parse");
    assert!(matches!(
        result,
        velesdb_core::FusionStrategy::RelativeScore { .. }
    ));
}

#[test]
fn test_unknown_strategy_returns_error() {
    let req = strat("nonexistent");
    let result = parse_fusion_strategy(Some(&req));
    assert!(
        result.is_err(),
        "unknown strategy must return Err (400 response)"
    );
}

#[test]
fn test_rrf_alias_case_insensitive() {
    for alias in ["rrf", "RRF", "Rrf"] {
        let req = strat(alias);
        let result =
            parse_fusion_strategy(Some(&req)).unwrap_or_else(|_| panic!("'{alias}' must parse"));
        assert!(matches!(result, velesdb_core::FusionStrategy::RRF { .. }));
    }
}
