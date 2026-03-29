//! Execution tests for LET clause (VelesQL v1.10 Phase 3).
//!
//! Validates that LET bindings are correctly evaluated and used by
//! ORDER BY during query execution. Tests cover:
//! - LET + ORDER BY interaction
//! - Weighted hybrid ordering
//! - Chained bindings
//! - Backward compatibility (no LET)
//! - Edge cases (unused bindings, WITH + LET, OFFSET + LET)

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::float_cmp
)]

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::collections::HashMap;
use tempfile::TempDir;

// ============================================================================
// Helper: create a 4-dim cosine collection with 20 points
// ============================================================================

fn setup_let_collection() -> (TempDir, Collection) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("let_col");
    let col = Collection::create(path, 4, DistanceMetric::Cosine).expect("create collection");

    let mut points = Vec::new();
    for i in 0u64..20 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        let v = vec![fi / 20.0, 1.0 - fi / 20.0, 0.5, 0.3];
        points.push(Point {
            id: i,
            vector: v,
            payload: Some(serde_json::json!({ "idx": i, "priority": 20 - i })),
            sparse_vectors: None,
        });
    }
    col.upsert(points).expect("upsert");
    (dir, col)
}

// ============================================================================
// A. LET binding used in ORDER BY
// ============================================================================

/// `LET s = similarity() ... ORDER BY s DESC` produces same ordering as
/// `ORDER BY similarity() DESC`.
#[test]
fn test_let_binding_in_order_by() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    // Baseline: ORDER BY similarity() DESC
    let baseline = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 5",
            &params,
        )
        .expect("baseline query");

    // LET version: ORDER BY s DESC
    let let_results = col
        .execute_query_str(
            "LET s = similarity() SELECT * FROM docs WHERE vector NEAR $v ORDER BY s DESC LIMIT 5",
            &params,
        )
        .expect("LET query");

    assert_eq!(baseline.len(), let_results.len());
    // Same ordering: same IDs in the same positions.
    let baseline_ids: Vec<u64> = baseline.iter().map(|r| r.point.id).collect();
    let let_ids: Vec<u64> = let_results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        baseline_ids, let_ids,
        "LET s = similarity() ORDER BY s should match ORDER BY similarity()"
    );
}

// ============================================================================
// B. Weighted hybrid scoring (LET changes ordering)
// ============================================================================

/// `LET hybrid = 0.01 * similarity() + 0.99 * priority ...`
/// This heavily weights `priority` (which decreases as id increases),
/// producing a DIFFERENT ordering than pure similarity-based ordering.
/// Similarity peaks around id=10 (middle), but priority peaks at id=0.
#[test]
fn test_let_weighted_hybrid_ordering() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    // Baseline: pure similarity ordering — top results cluster around id=10
    let baseline = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 10",
            &params,
        )
        .expect("baseline");

    // Hybrid: priority dominates — top results should be low-id (high priority)
    let hybrid = col
        .execute_query_str(
            "LET hybrid = 0.01 * similarity() + 0.99 * priority \
             SELECT * FROM docs WHERE vector NEAR $v ORDER BY hybrid DESC LIMIT 10",
            &params,
        )
        .expect("hybrid query");

    assert_eq!(hybrid.len(), 10);

    // Priority-dominated ordering should put low-id (high-priority) results first,
    // unlike similarity-dominated ordering which clusters around id~10.
    // Note: candidates come from HNSW search which limits to similar vectors, so
    // not all 20 points are available — the reorder happens within that candidate set.
    let baseline_ids: Vec<u64> = baseline.iter().map(|r| r.point.id).collect();
    let hybrid_ids: Vec<u64> = hybrid.iter().map(|r| r.point.id).collect();
    assert_ne!(
        baseline_ids, hybrid_ids,
        "Priority-dominated ordering should differ from similarity ordering"
    );
    // Verify that the highest-priority (lowest-id) candidate from the HNSW
    // result set is now first. With 0.99 weight on priority, the smallest id
    // in the candidate set should be first.
    let min_id_in_candidates = *hybrid_ids.iter().min().expect("non-empty results");
    assert_eq!(
        hybrid_ids[0], min_id_in_candidates,
        "First result should be the lowest-id (highest priority) candidate"
    );
}

// ============================================================================
// C. Chained bindings
// ============================================================================

/// `LET a = similarity() LET b = a * 2.0 ORDER BY b DESC` should produce
/// the same ranking as `ORDER BY similarity() DESC` (monotonic transform).
#[test]
fn test_let_chained_bindings() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    let baseline = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 5",
            &params,
        )
        .expect("baseline");

    let chained = col
        .execute_query_str(
            "LET a = similarity() LET b = a * 2.0 \
             SELECT * FROM docs WHERE vector NEAR $v ORDER BY b DESC LIMIT 5",
            &params,
        )
        .expect("chained query");

    let baseline_ids: Vec<u64> = baseline.iter().map(|r| r.point.id).collect();
    let chained_ids: Vec<u64> = chained.iter().map(|r| r.point.id).collect();
    assert_eq!(
        baseline_ids, chained_ids,
        "Monotonic transform should preserve ordering"
    );
}

// ============================================================================
// D. Literal binding (constant ordering)
// ============================================================================

/// `LET threshold = 0.8 ORDER BY threshold DESC` — constant value produces
/// stable but arbitrary ordering (all scores equal).
#[test]
fn test_let_literal_binding() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    let results = col
        .execute_query_str(
            "LET threshold = 0.8 \
             SELECT * FROM docs WHERE vector NEAR $v ORDER BY threshold DESC LIMIT 5",
            &params,
        )
        .expect("literal query");

    assert_eq!(results.len(), 5);
}

// ============================================================================
// E. Unused binding — no error
// ============================================================================

/// Binding defined but not referenced in ORDER BY should not cause errors.
#[test]
fn test_let_unused_binding_no_error() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    let results = col
        .execute_query_str(
            "LET unused = 0.5 \
             SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 5",
            &params,
        )
        .expect("unused binding should not error");

    assert_eq!(results.len(), 5);
}

// ============================================================================
// F. Backward compatibility — no LET
// ============================================================================

/// Query without LET works identically to before.
#[test]
fn test_let_backward_compat_no_let() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 5",
            &params,
        )
        .expect("no-LET query");

    assert_eq!(results.len(), 5);
}

// ============================================================================
// G. Edge cases — OFFSET + LET, WITH + LET
// ============================================================================

/// LET + OFFSET interaction works.
#[test]
fn test_let_with_offset_and_limit() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    let results = col
        .execute_query_str(
            "LET s = similarity() \
             SELECT * FROM docs WHERE vector NEAR $v ORDER BY s DESC LIMIT 3 OFFSET 2",
            &params,
        )
        .expect("LET + OFFSET");

    assert_eq!(results.len(), 3);
}

/// LET + WITH (mode='fast') combined.
#[test]
fn test_let_with_with_clause() {
    let (_dir, col) = setup_let_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    let results = col
        .execute_query_str(
            "LET s = similarity() \
             SELECT * FROM docs WHERE vector NEAR $v ORDER BY s DESC LIMIT 5 WITH (mode='fast')",
            &params,
        )
        .expect("LET + WITH");

    assert_eq!(results.len(), 5);
}

// ============================================================================
// H. Unit tests for evaluate_let_bindings
// ============================================================================

/// Evaluate a single literal binding.
#[test]
fn test_evaluate_let_bindings_single_literal() {
    use crate::collection::search::query::ordering::evaluate_let_bindings;
    use crate::velesql::{ArithmeticExpr, LetBinding};

    let bindings = vec![LetBinding {
        name: "x".to_string(),
        expr: ArithmeticExpr::Literal(0.42),
    }];

    let result = evaluate_let_bindings(&bindings, 0.5, None, None);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "x");
    assert!((result[0].1 - 0.42).abs() < 1e-5);
}

/// Chained: second binding references first.
#[test]
fn test_evaluate_let_bindings_chained() {
    use crate::collection::search::query::ordering::evaluate_let_bindings;
    use crate::velesql::{ArithmeticExpr, ArithmeticOp, LetBinding};

    let bindings = vec![
        LetBinding {
            name: "a".to_string(),
            expr: ArithmeticExpr::Literal(2.0),
        },
        LetBinding {
            name: "b".to_string(),
            expr: ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Variable("a".to_string())),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Literal(3.0)),
            },
        },
    ];

    let result = evaluate_let_bindings(&bindings, 0.0, None, None);
    assert_eq!(result.len(), 2);
    assert!((result[0].1 - 2.0).abs() < 1e-5, "a = 2.0");
    assert!(
        (result[1].1 - 6.0).abs() < 1e-5,
        "b = a * 3.0 = 6.0, got {}",
        result[1].1
    );
}

/// LET binding referencing similarity() uses search_score.
#[test]
fn test_evaluate_let_bindings_similarity() {
    use crate::collection::search::query::ordering::evaluate_let_bindings;
    use crate::velesql::{ArithmeticExpr, LetBinding, OrderByExpr};

    let bindings = vec![LetBinding {
        name: "s".to_string(),
        expr: ArithmeticExpr::Similarity(Box::new(OrderByExpr::SimilarityBare)),
    }];

    let result = evaluate_let_bindings(&bindings, 0.77, None, None);
    assert!((result[0].1 - 0.77).abs() < 1e-5);
}

/// Empty bindings produce empty result.
#[test]
fn test_evaluate_let_bindings_empty() {
    use crate::collection::search::query::ordering::evaluate_let_bindings;

    let result = evaluate_let_bindings(&[], 0.5, None, None);
    assert!(result.is_empty());
}
