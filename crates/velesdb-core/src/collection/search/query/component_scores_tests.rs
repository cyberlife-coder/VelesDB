//! Tests for component scores in SearchResult (VelesQL v1.10 Phase 2).
//!
//! Validates that individual search component scores (vector, BM25, graph, fused)
//! are tracked independently so ORDER BY arithmetic expressions like
//! `0.7 * vector_score + 0.3 * bm25_score` produce meaningful differentiation.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::float_cmp
)]

use crate::collection::search::query::ordering::{evaluate_arithmetic, ScoreContext};
use crate::point::{Point, SearchResult};
use crate::velesql::{ArithmeticExpr, ArithmeticOp};

// ============================================================================
// Helper: build SearchResult with optional component scores
// ============================================================================

fn make_result(id: u64, score: f32) -> SearchResult {
    SearchResult::new(Point::without_payload(id, vec![0.0; 4]), score)
}

fn make_result_with_components(
    id: u64,
    score: f32,
    components: Vec<(String, f32)>,
) -> SearchResult {
    let mut result = SearchResult::new(Point::without_payload(id, vec![0.0; 4]), score);
    result.component_scores = Some(components.into());
    result
}

// ============================================================================
// A. ScoreContext resolve_variable with component scores
// ============================================================================

/// Nominal: component score takes priority over fallback search_score.
#[test]
fn test_resolve_variable_prefers_component_over_search_score() {
    let components: smallvec::SmallVec<[(String, f32); 4]> = smallvec::smallvec![
        ("vector_score".to_string(), 0.9_f32),
        ("bm25_score".to_string(), 0.4_f32),
    ];
    let ctx = ScoreContext::with_components(0.65, None, Some(&components));

    let expr_vec = ArithmeticExpr::Variable("vector_score".to_string());
    let expr_bm25 = ArithmeticExpr::Variable("bm25_score".to_string());

    let vec_val = evaluate_arithmetic(&expr_vec, &ctx);
    let bm25_val = evaluate_arithmetic(&expr_bm25, &ctx);

    assert!(
        (vec_val - 0.9).abs() < 1e-5,
        "vector_score should resolve to component (0.9), got {vec_val}"
    );
    assert!(
        (bm25_val - 0.4).abs() < 1e-5,
        "bm25_score should resolve to component (0.4), got {bm25_val}"
    );
}

/// Nominal: when no component scores, falls back to search_score.
#[test]
fn test_resolve_variable_falls_back_to_search_score() {
    let ctx = ScoreContext::with_components(0.75, None, None);

    let expr = ArithmeticExpr::Variable("vector_score".to_string());
    let val = evaluate_arithmetic(&expr, &ctx);

    assert!(
        (val - 0.75).abs() < 1e-5,
        "Without components, vector_score should fall back to search_score (0.75), got {val}"
    );
}

/// Edge: component scores present but variable not in the list -> fallback.
#[test]
fn test_resolve_variable_missing_component_falls_back() {
    let components: smallvec::SmallVec<[(String, f32); 4]> =
        smallvec::smallvec![("vector_score".to_string(), 0.9_f32),];
    let ctx = ScoreContext::with_components(0.65, None, Some(&components));

    // bm25_score not in components -> should fall back to search_score
    let expr = ArithmeticExpr::Variable("bm25_score".to_string());
    let val = evaluate_arithmetic(&expr, &ctx);

    assert!(
        (val - 0.65).abs() < 1e-5,
        "Missing component should fall back to search_score (0.65), got {val}"
    );
}

/// Edge: empty component_scores vec treated like None.
#[test]
fn test_resolve_variable_empty_components_falls_back() {
    let components: smallvec::SmallVec<[(String, f32); 4]> = smallvec::smallvec![];
    let ctx = ScoreContext::with_components(0.80, None, Some(&components));

    let expr = ArithmeticExpr::Variable("vector_score".to_string());
    let val = evaluate_arithmetic(&expr, &ctx);

    assert!(
        (val - 0.80).abs() < 1e-5,
        "Empty components should fall back to search_score (0.80), got {val}"
    );
}

/// Edge: similarity() in arithmetic still resolves to search_score (fused),
/// NOT to a component score. This preserves backward compat.
#[test]
fn test_similarity_still_resolves_to_search_score() {
    let components: smallvec::SmallVec<[(String, f32); 4]> =
        smallvec::smallvec![("vector_score".to_string(), 0.9_f32),];
    let ctx = ScoreContext::with_components(0.65, None, Some(&components));

    let expr = ArithmeticExpr::Similarity(Box::new(crate::velesql::OrderByExpr::SimilarityBare));
    let val = evaluate_arithmetic(&expr, &ctx);

    assert!(
        (val - 0.65).abs() < 1e-5,
        "similarity() should always resolve to search_score (0.65), got {val}"
    );
}

/// Edge: fused_score resolves to search_score even when components exist.
#[test]
fn test_fused_score_resolves_to_search_score() {
    let components: smallvec::SmallVec<[(String, f32); 4]> = smallvec::smallvec![
        ("vector_score".to_string(), 0.9_f32),
        ("bm25_score".to_string(), 0.4_f32),
    ];
    let ctx = ScoreContext::with_components(0.65, None, Some(&components));

    let expr = ArithmeticExpr::Variable("fused_score".to_string());
    let val = evaluate_arithmetic(&expr, &ctx);

    assert!(
        (val - 0.65).abs() < 1e-5,
        "fused_score should resolve to search_score (0.65), got {val}"
    );
}

// ============================================================================
// B. Weighted ordering produces different results with real component scores
// ============================================================================

/// Nominal: `0.7 * vector_score + 0.3 * bm25_score` differs from plain similarity().
#[test]
fn test_weighted_components_differ_from_similarity() {
    // Result A: high vector, low BM25 => fused = 0.7
    // Result B: low vector, high BM25 => fused = 0.7
    // Without component scores, ORDER BY any weighted expression is identical.
    // WITH component scores, the ordering changes.

    let components_a: smallvec::SmallVec<[(String, f32); 4]> = smallvec::smallvec![
        ("vector_score".to_string(), 0.95_f32),
        ("bm25_score".to_string(), 0.10_f32),
    ];
    let components_b: smallvec::SmallVec<[(String, f32); 4]> = smallvec::smallvec![
        ("vector_score".to_string(), 0.30_f32),
        ("bm25_score".to_string(), 0.90_f32),
    ];

    // Both have the same fused score.
    let ctx_a = ScoreContext::with_components(0.70, None, Some(&components_a));
    let ctx_b = ScoreContext::with_components(0.70, None, Some(&components_b));

    // Expression: 0.7 * vector_score + 0.3 * bm25_score
    let expr = ArithmeticExpr::BinaryOp {
        left: Box::new(ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Literal(0.7)),
            op: ArithmeticOp::Mul,
            right: Box::new(ArithmeticExpr::Variable("vector_score".to_string())),
        }),
        op: ArithmeticOp::Add,
        right: Box::new(ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Literal(0.3)),
            op: ArithmeticOp::Mul,
            right: Box::new(ArithmeticExpr::Variable("bm25_score".to_string())),
        }),
    };

    let val_a = evaluate_arithmetic(&expr, &ctx_a);
    let val_b = evaluate_arithmetic(&expr, &ctx_b);

    // A: 0.7 * 0.95 + 0.3 * 0.10 = 0.665 + 0.030 = 0.695
    // B: 0.7 * 0.30 + 0.3 * 0.90 = 0.210 + 0.270 = 0.480
    assert!((val_a - 0.695).abs() < 1e-4, "Expected 0.695, got {val_a}");
    assert!((val_b - 0.480).abs() < 1e-4, "Expected 0.480, got {val_b}");
    assert!(
        (val_a - val_b).abs() > 0.1,
        "Weighted expression should produce DIFFERENT values for A and B, \
         but got A={val_a}, B={val_b}"
    );
}

/// Nominal: ORDER BY single component equals ORDER BY similarity() for pure vector.
#[test]
fn test_single_component_matches_similarity_for_pure_vector() {
    let components: smallvec::SmallVec<[(String, f32); 4]> =
        smallvec::smallvec![("vector_score".to_string(), 0.85_f32),];
    // For pure vector search, fused score == vector_score.
    let ctx = ScoreContext::with_components(0.85, None, Some(&components));

    let expr_vec = ArithmeticExpr::Variable("vector_score".to_string());
    let expr_sim =
        ArithmeticExpr::Similarity(Box::new(crate::velesql::OrderByExpr::SimilarityBare));

    let vec_val = evaluate_arithmetic(&expr_vec, &ctx);
    let sim_val = evaluate_arithmetic(&expr_sim, &ctx);

    assert!(
        (vec_val - sim_val).abs() < 1e-5,
        "For pure vector, vector_score ({vec_val}) should equal similarity() ({sim_val})"
    );
}

// ============================================================================
// C. SearchResult component_scores field
// ============================================================================

/// Nominal: component_scores defaults to None.
#[test]
fn test_component_scores_default_none() {
    let result = make_result(1, 0.5);
    assert!(
        result.component_scores.is_none(),
        "Default SearchResult should have no component scores"
    );
}

/// Nominal: component_scores can be set.
#[test]
fn test_component_scores_populated() {
    let result = make_result_with_components(
        1,
        0.7,
        vec![
            ("vector_score".to_string(), 0.9),
            ("bm25_score".to_string(), 0.3),
        ],
    );
    assert!(result.component_scores.is_some());
    let scores = result.component_scores.as_ref().expect("set above");
    assert_eq!(scores.len(), 2);
    assert!((scores[0].1 - 0.9).abs() < 1e-5);
    assert!((scores[1].1 - 0.3).abs() < 1e-5);
}

/// Edge: serialization backward compat — component_scores None should not
/// appear in JSON output.
#[test]
fn test_component_scores_none_not_serialized() {
    let result = make_result(1, 0.5);
    let json = serde_json::to_string(&result).expect("serialize");
    assert!(
        !json.contains("component_scores"),
        "None component_scores should not appear in JSON: {json}"
    );
}

/// Edge: deserialization backward compat — old JSON without component_scores
/// should parse successfully.
#[test]
fn test_backward_compat_deserialization() {
    let json = r#"{"point":{"id":1,"vector":[0.1,0.2,0.3,0.4]},"score":0.85}"#;
    let result: SearchResult = serde_json::from_str(json).expect("deserialize");
    assert_eq!(result.point.id, 1);
    assert!((result.score - 0.85).abs() < 1e-5);
    assert!(
        result.component_scores.is_none(),
        "Legacy JSON should deserialize with None component_scores"
    );
}

// ============================================================================
// D. ScoreContext integration with compare_by_order_columns
// ============================================================================

/// Nominal: ScoreContext::new (old API) creates context without components.
#[test]
fn test_score_context_new_has_no_components() {
    let ctx = ScoreContext::new(0.5, None);
    let expr = ArithmeticExpr::Variable("vector_score".to_string());
    let val = evaluate_arithmetic(&expr, &ctx);

    // Should fall back to search_score since no components.
    assert!(
        (val - 0.5).abs() < 1e-5,
        "ScoreContext::new should have no components, falling back to 0.5, got {val}"
    );
}

/// Edge: payload variable still resolves from payload, not component_scores.
#[test]
fn test_payload_variable_not_affected_by_components() {
    let payload = serde_json::json!({"price": 42.0});
    let components: smallvec::SmallVec<[(String, f32); 4]> =
        smallvec::smallvec![("vector_score".to_string(), 0.9_f32),];
    let ctx = ScoreContext::with_components(0.5, Some(&payload), Some(&components));

    let expr = ArithmeticExpr::Variable("price".to_string());
    let val = evaluate_arithmetic(&expr, &ctx);

    assert!(
        (val - 42.0).abs() < 1e-3,
        "Payload variable 'price' should resolve from payload (42.0), got {val}"
    );
}

// ============================================================================
// E. Integration: ordering.rs compare_by_order_columns uses component scores
// ============================================================================

/// Integration: Arithmetic ORDER BY with component_scores produces correct
/// differentiation between results that share the same fused score.
#[cfg(feature = "persistence")]
#[test]
fn test_order_by_arithmetic_uses_component_scores() {
    use crate::collection::types::Collection;
    use crate::distance::DistanceMetric;

    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(
        std::path::PathBuf::from(dir.path()),
        4,
        DistanceMetric::Cosine,
    )
    .expect("create");

    let points = vec![
        Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"category": "a"})),
            sparse_vectors: None,
        },
        Point {
            id: 2,
            vector: vec![0.9, 0.1, 0.0, 0.0],
            payload: Some(serde_json::json!({"category": "b"})),
            sparse_vectors: None,
        },
    ];
    col.upsert(points).expect("upsert");

    // Manually build results with component_scores to verify ordering uses them.
    let mut results = vec![
        make_result_with_components(
            1,
            0.70,
            vec![
                ("vector_score".to_string(), 0.95),
                ("bm25_score".to_string(), 0.10),
            ],
        ),
        make_result_with_components(
            2,
            0.70,
            vec![
                ("vector_score".to_string(), 0.30),
                ("bm25_score".to_string(), 0.90),
            ],
        ),
    ];

    // ORDER BY 0.7 * vector_score + 0.3 * bm25_score DESC
    let order_by = vec![crate::velesql::SelectOrderBy {
        expr: crate::velesql::OrderByExpr::Arithmetic(ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Literal(0.7)),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Variable("vector_score".to_string())),
            }),
            op: ArithmeticOp::Add,
            right: Box::new(ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Literal(0.3)),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Variable("bm25_score".to_string())),
            }),
        }),
        descending: true,
    }];

    let params = std::collections::HashMap::new();
    col.apply_order_by(&mut results, &order_by, &params)
        .expect("order_by");

    // Result 1 (vector-heavy): 0.7*0.95 + 0.3*0.10 = 0.695
    // Result 2 (bm25-heavy):   0.7*0.30 + 0.3*0.90 = 0.480
    // DESC -> result 1 should come first.
    assert_eq!(
        results[0].point.id, 1,
        "High-vector result should sort first in DESC with vector-weighted formula"
    );
    assert_eq!(results[1].point.id, 2);
}
