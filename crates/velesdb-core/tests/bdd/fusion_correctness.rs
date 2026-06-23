//! BDD tests for FUSION-correctness fixes (bugs #6, #10, #15, #16, #17).
//!
//! Each scenario drives the full pipeline (parse -> validate -> execute /
//! explain) through the public `Database` API so that validation-time rejects
//! and execution-time ranking are both exercised exactly as a caller sees them.
//!
//! The bugs being locked:
//! - #6: dense-NEAR + text-MATCH hybrid honors the FUSION `strategy` /
//!   `graph_weight` instead of always running plain weighted RRF.
//! - #10: FUSION weights are validated at validate-time (RSF must sum ~1.0,
//!   Weighted weights must be non-negative); EXPLAIN reports the strategy that
//!   actually executes.
//! - #15: NEAR_FUSED rejects `weighted`/`rsf`; the SQL parser maps
//!   `relative_score` -> Rsf and rejects unknown strategy names.
//! - #16: FUSION on a single fusable branch is a validate-time error.
//! - #17: `dense_weight`/`sparse_weight` long-name aliases are honored;
//!   unknown fusion keys are rejected.

use std::collections::BTreeMap;

use serde_json::json;
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{Database, DistanceMetric, Point};

use super::helpers::{create_test_db, execute_sql};

/// Builds a dim-2 collection 'docs' with text payloads + sparse vectors, used by
/// the hybrid-fusion scenarios.
fn setup_hybrid_docs(db: &Database) {
    db.create_vector_collection("docs", 2, DistanceMetric::Cosine)
        .expect("test: create docs");
    let vc = db.get_vector_collection("docs").expect("test: get docs");

    let mut s1 = BTreeMap::new();
    s1.insert(String::new(), SparseVector::new(vec![(10, 1.0)]));
    let mut s2 = BTreeMap::new();
    s2.insert(String::new(), SparseVector::new(vec![(10, 9.0)]));

    vc.upsert(vec![
        Point {
            id: 1,
            vector: vec![1.0, 0.0],
            payload: Some(json!({"content": "neural networks and learning"})),
            sparse_vectors: Some(s1),
        },
        Point {
            id: 2,
            vector: vec![0.0, 1.0],
            payload: Some(json!({"content": "neural learning systems"})),
            sparse_vectors: Some(s2),
        },
    ])
    .expect("test: upsert docs");
}

/// Returns the EXPLAIN fusion-strategy display string for a query.
fn explain_fusion_strategy(db: &Database, sql: &str) -> String {
    let query =
        velesdb_core::velesql::Parser::parse(sql).expect("test: parse explainable fusion query");
    let plan = db
        .explain_query(&query)
        .expect("test: explain fusion query");
    plan.fusion_info
        .expect("test: fusion_info present")
        .strategy
}

// ============================================================================
// #6 — strategy keyword honored on dense-NEAR + text-MATCH hybrid
// ============================================================================

/// `strategy='maximum'` must produce a different ranking than `strategy='rrf'`
/// on a dense-NEAR + text-MATCH hybrid (previously both ran plain RRF).
#[test]
fn bug6_match_hybrid_strategy_changes_ranking() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let rrf = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'rrf', k = 60)",
    )
    .expect("test: rrf hybrid");
    let maximum = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'maximum')",
    )
    .expect("test: maximum hybrid");

    let rrf_scores: Vec<f32> = rrf.iter().map(|r| r.score).collect();
    let max_scores: Vec<f32> = maximum.iter().map(|r| r.score).collect();
    assert_ne!(
        rrf_scores, max_scores,
        "maximum fusion must produce different scores than rrf"
    );
}

/// On the text-hybrid path, `graph_weight` must influence the text branch:
/// vector_weight=0.7/graph_weight=0.2 differs from graph_weight=0.3.
#[test]
fn bug6_graph_weight_affects_text_branch() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let low = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.2)",
    )
    .expect("test: weighted low graph");
    let high = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3)",
    )
    .expect("test: weighted high graph");

    let low_scores: Vec<f32> = low.iter().map(|r| r.score).collect();
    let high_scores: Vec<f32> = high.iter().map(|r| r.score).collect();
    assert_ne!(
        low_scores, high_scores,
        "graph_weight must change the text-branch weighting"
    );
}

// ============================================================================
// #10 — validate-time weight checks + honest EXPLAIN
// ============================================================================

/// RSF with weights that do not sum to ~1.0 must error at validate-time.
#[test]
fn bug10_rsf_bad_weight_sum_rejected() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let err = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND vector SPARSE_NEAR {10: 1.0} \
         LIMIT 2 USING FUSION(strategy = 'rsf', dense_w = 0.7, sparse_w = 0.7)",
    )
    .expect_err("test: RSF weights not summing to 1.0 must be rejected");
    assert!(
        err.to_string().contains("FUSION"),
        "expected FUSION validation marker, got: {err}"
    );
}

/// EXPLAIN of a NEAR+MATCH text-hybrid must report the strategy that actually
/// executes. After bug #6 the text-hybrid path honors `strategy='maximum'`
/// (score-level Maximum fusion), so EXPLAIN honestly reports `Maximum` — and a
/// plain `rrf` clause reports `RRF`. The EXPLAIN display and execution agree.
#[test]
fn bug10_explain_text_hybrid_matches_execution() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let maximum = explain_fusion_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'maximum')",
    );
    assert_eq!(maximum, "Maximum", "EXPLAIN must report executed Maximum");

    let rrf = explain_fusion_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'rrf', k = 60)",
    );
    assert_eq!(rrf, "RRF", "EXPLAIN must report executed RRF");
}

// ============================================================================
// #15 — NEAR_FUSED rejects weighted/rsf; relative_score alias; unknown reject
// ============================================================================

/// `NEAR_FUSED ... USING FUSION 'weighted'` must be a validate-time error.
#[test]
fn bug15_near_fused_weighted_rejected() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let err = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR_FUSED [[1.0, 0.0], [0.0, 1.0]] \
         USING FUSION 'weighted' LIMIT 2",
    )
    .expect_err("test: NEAR_FUSED weighted must be rejected");
    assert!(
        err.to_string().contains("FUSION"),
        "expected FUSION validation marker, got: {err}"
    );
}

/// SQL `strategy='relative_score'` maps to RSF (not silent RRF). With a
/// dense+sparse hybrid and balanced weights it runs and produces results.
#[test]
fn bug15_relative_score_alias_maps_to_rsf() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let strategy = explain_fusion_strategy(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND vector SPARSE_NEAR {10: 1.0} \
         LIMIT 2 USING FUSION(strategy = 'relative_score', dense_w = 0.5, sparse_w = 0.5)",
    );
    assert_eq!(
        strategy, "RSF",
        "relative_score alias must resolve to RSF, not RRF"
    );
}

/// An unknown FUSION strategy name must be rejected, not silently run RRF.
#[test]
fn bug15_unknown_strategy_rejected() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let err = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'nonsense')",
    )
    .expect_err("test: unknown strategy must be rejected");
    assert!(
        err.to_string().contains("strategy") || err.to_string().contains("FUSION"),
        "expected unknown-strategy marker, got: {err}"
    );
}

// ============================================================================
// #16 — FUSION on a single fusable branch is a validate-time error
// ============================================================================

/// `similarity()`-only query with USING FUSION has no second branch -> reject.
#[test]
fn bug16_similarity_only_fusion_rejected() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let err = execute_sql(
        &db,
        "SELECT * FROM docs WHERE similarity(vector, [1.0, 0.0]) > 0.5 \
         LIMIT 2 USING FUSION(strategy = 'maximum')",
    )
    .expect_err("test: similarity-only FUSION must be rejected");
    assert!(
        err.to_string().contains("FUSION"),
        "expected FUSION applicability marker, got: {err}"
    );
}

/// Pure NEAR (single branch) with USING FUSION -> reject.
#[test]
fn bug16_pure_near_fusion_rejected() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let err = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] \
         LIMIT 2 USING FUSION(strategy = 'rrf', k = 60)",
    )
    .expect_err("test: pure NEAR FUSION must be rejected");
    assert!(
        err.to_string().contains("FUSION"),
        "expected FUSION applicability marker, got: {err}"
    );
}

/// A genuine two-branch query with USING FUSION still validates and runs.
#[test]
fn bug16_two_branch_fusion_allowed() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND content MATCH 'learning' \
         LIMIT 2 USING FUSION(strategy = 'rrf', k = 60)",
    )
    .expect("test: two-branch FUSION must be accepted");
    assert!(!results.is_empty(), "two-branch fusion must return results");
}

// ============================================================================
// #17 — dense_weight/sparse_weight long names + unknown-key reject
// ============================================================================

/// The documented RSF example with `dense_weight=0.7, sparse_weight=0.3`
/// resolves those weights (not 50/50) — proven via a sparse-heavy ranking.
#[test]
fn bug17_long_name_weights_honored() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    // Strongly dense-favored vs strongly sparse-favored should differ.
    let dense_heavy = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND vector SPARSE_NEAR {10: 1.0} \
         LIMIT 2 USING FUSION(strategy = 'rsf', dense_weight = 0.9, sparse_weight = 0.1)",
    )
    .expect("test: dense-heavy rsf");
    let sparse_heavy = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND vector SPARSE_NEAR {10: 1.0} \
         LIMIT 2 USING FUSION(strategy = 'rsf', dense_weight = 0.1, sparse_weight = 0.9)",
    )
    .expect("test: sparse-heavy rsf");

    let dense_order: Vec<u64> = dense_heavy.iter().map(|r| r.point.id).collect();
    let sparse_order: Vec<u64> = sparse_heavy.iter().map(|r| r.point.id).collect();
    assert_ne!(
        dense_order, sparse_order,
        "long-name dense_weight/sparse_weight must influence ranking (not 50/50)"
    );
}

/// A misspelled fusion key must be rejected, not silently ignored.
#[test]
fn bug17_unknown_fusion_key_rejected() {
    let (_dir, db) = create_test_db();
    setup_hybrid_docs(&db);

    let err = execute_sql(
        &db,
        "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0] AND vector SPARSE_NEAR {10: 1.0} \
         LIMIT 2 USING FUSION(strategy = 'rsf', dense_wieght = 0.7, sparse_weight = 0.3)",
    )
    .expect_err("test: misspelled fusion key must be rejected");
    assert!(
        err.to_string().contains("FUSION") || err.to_string().contains("dense_wieght"),
        "expected unknown-fusion-key marker, got: {err}"
    );
}
