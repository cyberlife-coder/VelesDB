//! BDD tests for the two "Weighted" fusion bugs plus the correct engine-level
//! Weighted behavior.
//!
//! Two regression locks document live bugs by asserting the CURRENT (buggy)
//! behavior; their doc-comments state what the CORRECT behavior would be:
//! - Bug A (`sparse_dispatch.rs:204`): SQL `USING FUSION(strategy='weighted')`
//!   is silently downgraded to `RRF{k:60}` — it never reaches the engine's
//!   `FusionStrategy::Weighted`.
//! - Bug B (`score_fusion/mod.rs:248`): the per-result `ScoreFusionMethod::Weighted`
//!   combiner uses equal weights, so it is identical to `Average`.
//!
//! The first two tests pin the CORRECT engine-level `FusionStrategy::weighted`
//! ranking + its validation, so the contrast with the bugs is explicit.

use std::collections::BTreeMap;

use serde_json::json;
use velesdb_core::collection::search::query::score_fusion::{ScoreBreakdown, ScoreFusionMethod};
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{DistanceMetric, FusionStrategy, Point};

use super::helpers::{approx_eq, create_test_db, execute_sql};

/// CORRECT engine-level Weighted fusion: per-doc combined =
/// avg_w*avg + max_w*max + hit_w*(hits/total_queries).
///
/// Ground truth (hand-computed, weights 0.6/0.3/0.1):
/// branch0=[(1,0.95),(2,0.80)], branch1=[(2,0.95),(1,0.70)].
/// doc1: avg=(0.95+0.70)/2=0.825, max=0.95, hit=2/2=1.0 ->
///       0.6*0.825+0.3*0.95+0.1*1.0 = 0.880.
/// doc2: avg=(0.80+0.95)/2=0.875, max=0.95, hit=1.0 -> 0.910.
/// Scores are strictly distinct, so id ORDER [2,1] is deterministic.
#[test]
fn test_engine_weighted_fusion_correct_ranking() {
    let strategy = FusionStrategy::weighted(0.6, 0.3, 0.1).expect("test: valid weighted strategy");

    let fused = strategy
        .fuse(vec![vec![(1, 0.95), (2, 0.80)], vec![(2, 0.95), (1, 0.70)]])
        .expect("test: fuse weighted");

    let ids: Vec<u64> = fused.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![2, 1], "doc2 (0.910) ranks above doc1 (0.880)");

    let score = |target: u64| fused.iter().find(|(id, _)| *id == target).map(|(_, s)| *s);
    assert!(
        approx_eq(score(2).expect("test: doc2 present"), 0.910, 1e-4),
        "doc2 weighted score must be 0.910"
    );
    assert!(
        approx_eq(score(1).expect("test: doc1 present"), 0.880, 1e-4),
        "doc1 weighted score must be 0.880"
    );
}

/// `FusionStrategy::weighted` rejects weights that do not sum to 1.0 and any
/// negative weight; `fuse` re-validates so direct enum-literal construction
/// cannot bypass validation.
///
/// Ground truth: sum 0.5+0.3+0.3=1.1 (>1.0±0.001) -> Err; a -0.1 weight -> Err;
/// a literal `Weighted{0.5,0.3,0.3}` fused over a non-empty input -> Err.
#[test]
fn test_engine_weighted_validation_rejects_bad_weights() {
    assert!(
        FusionStrategy::weighted(0.5, 0.3, 0.3).is_err(),
        "sum 1.1 must be rejected"
    );
    assert!(
        FusionStrategy::weighted(-0.1, 0.6, 0.5).is_err(),
        "negative weight must be rejected"
    );

    let bypass = FusionStrategy::Weighted {
        avg_weight: 0.5,
        max_weight: 0.3,
        hit_weight: 0.3,
    };
    assert!(
        bypass.fuse(vec![vec![(1u64, 0.9f32)]]).is_err(),
        "fuse must re-validate the literal-constructed bad-sum Weighted"
    );
}

/// REGRESSION LOCK — Bug B (`score_fusion/mod.rs:248`).
///
/// `ScoreFusionMethod::Weighted` uses equal weights (1/n per component), so for
/// a breakdown with vector_similarity=0.9 and sparse_score=0.3 it computes
/// 0.9*0.5 + 0.3*0.5 = 0.6 — identical to `Average` = (0.9+0.3)/2 = 0.6.
///
/// CORRECT behavior: a TRUE weighted combiner (e.g. w_vec=0.75, w_sparse=0.25)
/// would yield 0.9*0.75 + 0.3*0.25 = 0.75, NOT 0.6, and would differ from
/// Average. This test PASSES against the current buggy equal-weight behavior.
#[test]
fn test_score_fusion_weighted_equals_average_bug_b() {
    let breakdown = ScoreBreakdown {
        vector_similarity: Some(0.9),
        sparse_score: Some(0.3),
        ..Default::default()
    };

    let weighted = ScoreFusionMethod::Weighted.combine(&breakdown);
    let average = ScoreFusionMethod::Average.combine(&breakdown);

    assert!(
        approx_eq(weighted, 0.6, 1e-6),
        "buggy Weighted combiner yields equal-weight mean 0.6"
    );
    assert!(
        approx_eq(weighted, average, 1e-6),
        "Bug B: Weighted is indistinguishable from Average"
    );
}

/// Builds the dim-2 hybrid collection 'wb' used by the Bug A lock:
/// id1 dense [1,0] / sparse {10:1.0}; id2 dense [0,1] / sparse {10:9.0}.
fn setup_wb_collection(db: &velesdb_core::Database) {
    db.create_vector_collection("wb", 2, DistanceMetric::Cosine)
        .expect("test: create wb");
    let vc = db.get_vector_collection("wb").expect("test: get wb");

    let mut sparse1 = BTreeMap::new();
    sparse1.insert(String::new(), SparseVector::new(vec![(10, 1.0)]));
    let mut sparse2 = BTreeMap::new();
    sparse2.insert(String::new(), SparseVector::new(vec![(10, 9.0)]));

    vc.upsert(vec![
        Point {
            id: 1,
            vector: vec![1.0, 0.0],
            payload: Some(json!({"content": "doc one"})),
            sparse_vectors: Some(sparse1),
        },
        Point {
            id: 2,
            vector: vec![0.0, 1.0],
            payload: Some(json!({"content": "doc two"})),
            sparse_vectors: Some(sparse2),
        },
    ])
    .expect("test: upsert wb");
}

/// REGRESSION LOCK — Bug A (`sparse_dispatch.rs:204`).
///
/// SQL `USING FUSION(strategy='weighted')` is silently downgraded to
/// `RRF{k:60}` (the only SQL path reaching the engine fusion). With dense
/// ranking id1 first and sparse ranking id2 first, RRF is a symmetric tie:
/// each doc = 1/(60+1) + 1/(60+2) = 0.0325225. We assert the returned scores
/// equal that RRF value, proving the SQL ran RRF, NOT engine Weighted.
///
/// CORRECT behavior: a real 'weighted' SQL strategy would invoke
/// `FusionStrategy::Weighted` and produce the avg/max/hit combination instead
/// of an RRF tie. The tie means id order is non-deterministic — we assert
/// scores only, never id order.
#[test]
fn test_sql_weighted_strategy_downgrades_to_rrf_bug_a() {
    let (_dir, db) = create_test_db();
    setup_wb_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM wb WHERE vector NEAR [1.0, 0.0] AND vector SPARSE_NEAR {10: 1.0} \
         LIMIT 2 USING FUSION(strategy = 'weighted')",
    )
    .expect("test: execute hybrid weighted query");

    assert_eq!(
        results.len(),
        2,
        "both docs surface across the two branches"
    );
    for r in &results {
        assert!(
            approx_eq(r.score, 0.032_522_5, 1e-4),
            "Bug A: SQL 'weighted' ran RRF{{60}} -> symmetric tie 0.0325225, got {}",
            r.score
        );
    }
}
