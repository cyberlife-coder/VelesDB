#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
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

    breakdown.compute_final(&ScoreCombineStrategy::Average);
    assert!((breakdown.final_score - 0.8).abs() < 0.001);
}

#[test]
fn test_fusion_strategy_maximum() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.7);

    breakdown.compute_final(&ScoreCombineStrategy::Maximum);
    assert!((breakdown.final_score - 0.9).abs() < 0.001);
}

#[test]
fn test_fusion_strategy_minimum() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.7);

    breakdown.compute_final(&ScoreCombineStrategy::Minimum);
    assert!((breakdown.final_score - 0.7).abs() < 0.001);
}

#[test]
fn test_fusion_strategy_product() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.8);

    breakdown.compute_final(&ScoreCombineStrategy::Product);
    assert!((breakdown.final_score - 0.72).abs() < 0.001);
}

#[test]
fn test_fusion_with_metadata_boost() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.8)
        .with_metadata_boost(1.5);

    breakdown.compute_final(&ScoreCombineStrategy::Average);
    assert!((breakdown.final_score - 1.2).abs() < 0.001);
}

#[test]
fn test_fusion_with_multiple_boosts() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.8)
        .with_metadata_boost(1.2)
        .with_recency_boost(1.1);

    breakdown.compute_final(&ScoreCombineStrategy::Average);
    assert!((breakdown.final_score - 1.056).abs() < 0.01);
}

#[test]
fn test_fusion_with_custom_boost() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.5)
        .with_custom_boost("popularity", 2.0);

    breakdown.compute_final(&ScoreCombineStrategy::Average);
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
    bd.compute_final(&ScoreCombineStrategy::Average);

    let result = ScoredResult::with_breakdown(1, bd);
    assert_eq!(result.id, 1);
    assert!(result.score_breakdown.vector_similarity.is_some());
}

#[test]
fn test_score_combine_strategy_as_str() {
    assert_eq!(ScoreCombineStrategy::Weighted.as_str(), "weighted");
    assert_eq!(ScoreCombineStrategy::Maximum.as_str(), "maximum");
    assert_eq!(ScoreCombineStrategy::Minimum.as_str(), "minimum");
    assert_eq!(ScoreCombineStrategy::Product.as_str(), "product");
    assert_eq!(ScoreCombineStrategy::Average.as_str(), "average");
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
    breakdown.compute_final(&ScoreCombineStrategy::Average);

    let result = ScoredResult::with_breakdown(42, breakdown)
        .with_payload(serde_json::json!({"title": "Test"}));

    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("score_breakdown"));
    assert!(json.contains("title"));
    assert!(json.contains("Test"));
}

// ============================================================================
// PathScorer Tests (EPIC-049 US-002)
// ============================================================================

#[test]
fn test_path_scorer_empty_path_returns_one() {
    let scorer = PathScorer::new();
    let path: Vec<(u64, u64, &str)> = vec![];
    assert!((scorer.score_path(&path) - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_path_scorer_shorter_path_scores_higher() {
    let scorer = PathScorer::new().with_decay(0.8);

    // One hop
    let short_path = vec![(1, 2, "KNOWS")];
    // Three hops
    let long_path = vec![(1, 2, "KNOWS"), (2, 3, "KNOWS"), (3, 4, "KNOWS")];

    let short_score = scorer.score_path(&short_path);
    let long_score = scorer.score_path(&long_path);

    assert!(
        short_score > long_score,
        "Short path {short_score} should score higher than long path {long_score}"
    );
}

#[test]
fn test_path_scorer_rel_type_weights() {
    let scorer = PathScorer::new()
        .with_decay(1.0) // No decay, isolate rel_type effect
        .with_rel_weight("AUTHORED", 1.0)
        .with_rel_weight("MENTIONS", 0.5);

    let authored_path = vec![(1, 2, "AUTHORED")];
    let mentions_path = vec![(1, 2, "MENTIONS")];

    let authored_score = scorer.score_path(&authored_path);
    let mentions_score = scorer.score_path(&mentions_path);

    assert!(
        authored_score > mentions_score,
        "AUTHORED {authored_score} should score higher than MENTIONS {mentions_score}"
    );
    assert!((authored_score - 1.0).abs() < f32::EPSILON);
    assert!((mentions_score - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_path_scorer_default_weight() {
    let scorer = PathScorer::new().with_decay(1.0).with_default_weight(0.7);

    let path = vec![(1, 2, "UNKNOWN_TYPE")];
    let score = scorer.score_path(&path);

    assert!((score - 0.7).abs() < f32::EPSILON);
}

#[test]
fn test_path_scorer_score_rel_types() {
    let scorer = PathScorer::new()
        .with_decay(0.8)
        .with_rel_weight("A", 1.0)
        .with_rel_weight("B", 0.5);

    let score = scorer.score_rel_types(&["A", "B"]);

    // First hop: 0.8 * 1.0 = 0.8
    // Second hop: 0.8^2 * 0.5 = 0.64 * 0.5 = 0.32
    // Product: 0.8 * 0.32 = 0.256
    assert!((score - 0.256).abs() < 0.001);
}

#[test]
fn test_path_scorer_score_length() {
    let scorer = PathScorer::new().with_decay(0.5);

    assert!((scorer.score_length(0) - 1.0).abs() < f32::EPSILON);
    assert!((scorer.score_length(1) - 0.5).abs() < f32::EPSILON);
    assert!((scorer.score_length(2) - 0.25).abs() < f32::EPSILON);
    assert!((scorer.score_length(3) - 0.125).abs() < f32::EPSILON);
}

#[test]
fn test_path_scorer_combined_with_breakdown() {
    let scorer = PathScorer::new().with_decay(0.8);

    // Simulate a hybrid query result
    let path_score = scorer.score_length(2); // 0.8^2 = 0.64

    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_path(path_score);

    breakdown.compute_final(&ScoreCombineStrategy::Average);

    // Average of 0.9 and 0.64 = 0.77
    assert!((breakdown.final_score - 0.77).abs() < 0.01);
}

// ============================================================================
// Metadata Boost Tests (EPIC-049 US-003)
// ============================================================================

#[test]
fn test_field_boost_basic() {
    let boost = FieldBoost::new("importance").with_scale(0.1);

    let doc = serde_json::json!({"importance": 5.0});
    let result = boost.compute(&doc);

    // 1.0 + 5.0 * 0.1 = 1.5
    assert!((result - 1.5).abs() < f32::EPSILON);
}

#[test]
fn test_field_boost_missing_field() {
    let boost = FieldBoost::new("importance");

    let doc = serde_json::json!({"other_field": 10});
    let result = boost.compute(&doc);

    // Missing field -> 0.0 value -> 1.0 boost
    assert!((result - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_field_boost_bounds() {
    let boost = FieldBoost::new("score")
        .with_scale(1.0)
        .with_bounds(0.5, 2.0);

    // Test max bound
    let high_doc = serde_json::json!({"score": 100.0});
    assert!((boost.compute(&high_doc) - 2.0).abs() < f32::EPSILON);

    // Test min bound
    let low_doc = serde_json::json!({"score": -10.0});
    assert!((boost.compute(&low_doc) - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_recency_boost_brand_new() {
    let boost = RecencyBoost::new("created_at", 30.0, 1.5);

    // Brand new document (now) - use Unix timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let doc = serde_json::json!({"created_at": now});
    let result = boost.compute(&doc);

    // Should be close to max_boost for brand new
    assert!(result > 1.4 && result <= 1.5);
}

#[test]
fn test_recency_boost_missing_field() {
    let boost = RecencyBoost::default();

    let doc = serde_json::json!({"other_field": "value"});
    let result = boost.compute(&doc);

    // Missing timestamp -> neutral boost
    assert!((result - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_recency_boost_unix_timestamp() {
    let boost = RecencyBoost::new("timestamp", 30.0, 1.5);

    // Current Unix timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let doc = serde_json::json!({"timestamp": now});
    let result = boost.compute(&doc);

    // Should be close to max_boost
    assert!(result > 1.4);
}

#[test]
fn test_composite_boost_multiply() {
    let composite = CompositeBoost::new(BoostCombination::Multiply)
        .with_boost(FieldBoost::new("a").with_scale(1.0).with_bounds(0.0, 10.0))
        .with_boost(FieldBoost::new("b").with_scale(1.0).with_bounds(0.0, 10.0));

    // a=1 -> boost 2.0, b=0.5 -> boost 1.5
    let doc = serde_json::json!({"a": 1.0, "b": 0.5});
    let result = composite.compute(&doc);

    // 2.0 * 1.5 = 3.0
    assert!((result - 3.0).abs() < f32::EPSILON);
}

#[test]
fn test_composite_boost_add() {
    let composite = CompositeBoost::new(BoostCombination::Add)
        .with_boost(FieldBoost::new("a").with_scale(1.0).with_bounds(0.0, 10.0))
        .with_boost(FieldBoost::new("b").with_scale(1.0).with_bounds(0.0, 10.0));

    // a=1 -> boost 2.0, b=0.5 -> boost 1.5
    let doc = serde_json::json!({"a": 1.0, "b": 0.5});
    let result = composite.compute(&doc);

    // 2.0 + 1.5 - 1 = 2.5 (subtract n-1 to keep neutral at 1.0)
    assert!((result - 2.5).abs() < f32::EPSILON);
}

#[test]
fn test_composite_boost_max() {
    let composite = CompositeBoost::new(BoostCombination::Max)
        .with_boost(FieldBoost::new("a").with_scale(1.0).with_bounds(0.0, 10.0))
        .with_boost(FieldBoost::new("b").with_scale(1.0).with_bounds(0.0, 10.0));

    // a=1 -> boost 2.0, b=0.5 -> boost 1.5
    let doc = serde_json::json!({"a": 1.0, "b": 0.5});
    let result = composite.compute(&doc);

    // max(2.0, 1.5) = 2.0
    assert!((result - 2.0).abs() < f32::EPSILON);
}

#[test]
fn test_composite_boost_empty() {
    let composite = CompositeBoost::new(BoostCombination::Multiply);
    let doc = serde_json::json!({});
    let result = composite.compute(&doc);

    // Empty composite -> neutral
    assert!((result - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_boost_function_names() {
    let recency = RecencyBoost::default();
    let field = FieldBoost::default();
    let composite = CompositeBoost::default();

    assert_eq!(recency.name(), "recency");
    assert_eq!(field.name(), "field");
    assert_eq!(composite.name(), "composite");
}

// ============================================================================
// Score Explanation API Tests (EPIC-049 US-005)
// ============================================================================

#[test]
fn test_explain_vector_only() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.85);
    breakdown.compute_final(&ScoreCombineStrategy::Average);

    let explanation = breakdown.explain(&ScoreCombineStrategy::Average);

    assert!((explanation.final_score - 0.85).abs() < f32::EPSILON);
    assert_eq!(explanation.strategy, "average");
    assert_eq!(explanation.components.len(), 1);
    assert_eq!(explanation.components[0].name, "vector_similarity");
    assert!((explanation.components[0].value - 0.85).abs() < f32::EPSILON);
}

#[test]
fn test_explain_hybrid_query() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.9)
        .with_graph(0.7)
        .with_path(0.8);
    breakdown.compute_final(&ScoreCombineStrategy::Average);

    let explanation = breakdown.explain(&ScoreCombineStrategy::Average);

    // 3 weighted components + none for boosts
    assert_eq!(explanation.components.len(), 3);

    // Check weights are equal (1/3 each)
    for comp in &explanation.components {
        if let Some(weight) = comp.weight {
            assert!((weight - 0.333).abs() < 0.01);
        }
    }
}

#[test]
fn test_explain_with_boosts() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.9)
        .with_metadata_boost(1.2)
        .with_recency_boost(1.1);
    breakdown.compute_final(&ScoreCombineStrategy::Average);

    let explanation = breakdown.explain(&ScoreCombineStrategy::Average);

    // 1 weighted component (vector) + 2 multipliers
    assert_eq!(explanation.components.len(), 3);

    // Find boost components - they should have no weight
    let boosts: Vec<_> = explanation
        .components
        .iter()
        .filter(|c| c.weight.is_none())
        .collect();
    assert_eq!(boosts.len(), 2);
}

#[test]
fn test_explain_human_readable_format() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.92).with_graph(0.75);
    breakdown.compute_final(&ScoreCombineStrategy::Average);

    let explanation = breakdown.explain(&ScoreCombineStrategy::Average);

    // Should contain final score line
    assert!(explanation.human_readable.contains("Final score:"));
    // Should contain component names
    assert!(explanation.human_readable.contains("vector_similarity"));
    assert!(explanation.human_readable.contains("graph_distance"));
    // Should contain weight percentages
    assert!(explanation.human_readable.contains("weight:"));
}

#[test]
fn test_explain_custom_boosts() {
    let mut breakdown = ScoreBreakdown::new()
        .with_vector(0.8)
        .with_custom_boost("popularity", 1.5);
    breakdown.compute_final(&ScoreCombineStrategy::Average);

    let explanation = breakdown.explain(&ScoreCombineStrategy::Average);

    // Should include custom boost
    let custom = explanation
        .components
        .iter()
        .find(|c| c.name.starts_with("custom:"));
    assert!(custom.is_some());
    assert!(custom.unwrap().description.contains("popularity"));
}

#[test]
fn test_explanation_json_serialization() {
    let mut breakdown = ScoreBreakdown::new().with_vector(0.9).with_graph(0.8);
    breakdown.compute_final(&ScoreCombineStrategy::Average);

    let explanation = breakdown.explain(&ScoreCombineStrategy::Average);
    let json = serde_json::to_string(&explanation).unwrap();

    assert!(json.contains("final_score"));
    assert!(json.contains("strategy"));
    assert!(json.contains("components"));
    assert!(json.contains("human_readable"));
}
