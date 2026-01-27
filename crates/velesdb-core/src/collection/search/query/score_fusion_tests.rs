//! Tests for `score_fusion` module - Multi-score fusion strategies.

use super::score_fusion::*;

#[test]
fn test_score_breakdown_new() {
    let breakdown = ScoreBreakdown::new();
    assert!(breakdown.vector_similarity.is_none());
    assert!(breakdown.graph_distance.is_none());
    assert!((breakdown.final_score - 0.0).abs() < f32::EPSILON);
}

#[test]
fn test_score_breakdown_from_vector() {
    let breakdown = ScoreBreakdown::from_vector(0.85);
    assert_eq!(breakdown.vector_similarity, Some(0.85));
    assert!((breakdown.final_score - 0.85).abs() < f32::EPSILON);
}

#[test]
fn test_score_breakdown_builder() {
    let breakdown = ScoreBreakdown::new()
        .with_vector(0.9)
        .with_graph(0.8)
        .with_metadata_boost(1.2);

    assert_eq!(breakdown.vector_similarity, Some(0.9));
    assert_eq!(breakdown.graph_distance, Some(0.8));
    assert_eq!(breakdown.metadata_boost, Some(1.2));
}

#[test]
fn test_score_breakdown_components() {
    let breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.8);

    let components = breakdown.components();
    assert_eq!(components.len(), 2);
    assert!(components.contains(&("vector_similarity", 0.9)));
    assert!(components.contains(&("graph_distance", 0.8)));
}

#[test]
fn test_fusion_strategy_average() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.7);

    breakdown.compute_final(&FusionStrategy::Average);
    assert!((breakdown.final_score - 0.8).abs() < 0.001);
}

#[test]
fn test_fusion_strategy_maximum() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.7);

    breakdown.compute_final(&FusionStrategy::Maximum);
    assert!((breakdown.final_score - 0.9).abs() < 0.001);
}

#[test]
fn test_fusion_strategy_minimum() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.7);

    breakdown.compute_final(&FusionStrategy::Minimum);
    assert!((breakdown.final_score - 0.7).abs() < 0.001);
}

#[test]
fn test_fusion_strategy_product() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.8);

    breakdown.compute_final(&FusionStrategy::Product);
    assert!((breakdown.final_score - 0.72).abs() < 0.001);
}

#[test]
fn test_fusion_with_metadata_boost() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.8)
        .with_metadata_boost(1.5);

    breakdown.compute_final(&FusionStrategy::Average);
    assert!((breakdown.final_score - 1.2).abs() < 0.001);
}

#[test]
fn test_fusion_with_multiple_boosts() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.8)
        .with_metadata_boost(1.2)
        .with_recency_boost(1.1);

    breakdown.compute_final(&FusionStrategy::Average);
    assert!((breakdown.final_score - 1.056).abs() < 0.01);
}

#[test]
fn test_fusion_with_custom_boost() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.5)
        .with_custom_boost("popularity", 2.0);

    breakdown.compute_final(&FusionStrategy::Average);
    assert!((breakdown.final_score - 1.0).abs() < 0.001);
}

#[test]
fn test_scored_result_new() {
    let result = ScoredResult::new(42, 0.95);
    assert_eq!(result.id, 42);
    assert!((result.score - 0.95).abs() < f32::EPSILON);
    assert!(result.payload.is_none());
}

#[test]
fn test_scored_result_with_breakdown() {
    let breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.8);

    let mut bd = breakdown.clone();
    bd.compute_final(&FusionStrategy::Average);

    let result = ScoredResult::with_breakdown(1, bd);
    assert_eq!(result.id, 1);
    assert!(result.score_breakdown.vector_similarity.is_some());
}

#[test]
fn test_fusion_strategy_as_str() {
    assert_eq!(FusionStrategy::Rrf.as_str(), "rrf");
    assert_eq!(FusionStrategy::Weighted.as_str(), "weighted");
    assert_eq!(FusionStrategy::Maximum.as_str(), "maximum");
    assert_eq!(FusionStrategy::Average.as_str(), "average");
}

#[test]
fn test_score_breakdown_json_serialization() {
    let breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.8);

    let json = serde_json::to_string(&breakdown).unwrap();
    assert!(json.contains("vector_similarity"));
    assert!(json.contains("0.9"));
    assert!(!json.contains("path_score"));
}

#[test]
fn test_scored_result_json_serialization() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9);
    breakdown.compute_final(&FusionStrategy::Average);

    let result = ScoredResult::with_breakdown(42, breakdown)
        .with_payload(serde_json::json!({"title": "Test"}));

    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("score_breakdown"));
    assert!(json.contains("title"));
    assert!(json.contains("Test"));
}
