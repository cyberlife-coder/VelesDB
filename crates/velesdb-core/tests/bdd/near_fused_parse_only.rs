//! BDD tests for the two `NEAR_FUSED` surfaces — the SQL surface and the
//! engine-level multi-vector fusion API — which must agree.
//!
//! Contract under test (two surfaces):
//!
//! 1. **VelesQL `NEAR_FUSED` executes real multi-vector fusion.** The grammar
//!    (`grammar.pest` `vector_fused_search`) and parser
//!    (`condition_vectors.rs::parse_vector_fused_search`) build a
//!    `Condition::VectorFusedSearch`; `query/fused_dispatch.rs::dispatch_fused_query`
//!    routes it to `Collection::multi_query_search` (the same fusion as the
//!    engine API), honoring `USING FUSION` and any residual metadata filter.
//!
//! 2. **Engine-API parity.** `VectorCollection::multi_query_search` performs the
//!    same fusion directly; the SQL and engine surfaces agree (diagonal id3 top).

use velesdb_core::{Database, FusionStrategy, Point};

use super::helpers::{create_test_db, execute_sql, result_ids};

/// dim-2 fixture: id1 axis-x, id2 axis-y, id3 the diagonal near both axes.
/// Inserted in storage order 1,2,3 so an unranked scan yields {1,2,3}.
fn setup_nf(db: &Database, name: &str) {
    db.create_vector_collection(name, 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create collection");
    let vc = db
        .get_vector_collection(name)
        .expect("test: get collection");
    let points = vec![
        Point::new(1, vec![1.0, 0.0], Some(serde_json::json!({"content": "x"}))),
        Point::new(2, vec![0.0, 1.0], Some(serde_json::json!({"content": "y"}))),
        Point::new(
            3,
            vec![0.7, 0.7],
            Some(serde_json::json!({"content": "xy"})),
        ),
    ];
    vc.upsert(points).expect("test: upsert");
}

// ============================================================================
// Surface 1 — VelesQL NEAR_FUSED parses and executes real fusion
// ============================================================================

/// LOCK: `NEAR_FUSED [[..],[..]]` PARSES into `Condition::VectorFusedSearch`
/// (grammar `vector_fused_search`; `parse_vector_fused_search`). Ground truth:
/// `Parser::parse` returns Ok for a well-formed two-vector NEAR_FUSED clause.
#[test]
fn near_fused_two_vectors_parses_ok() {
    let sql = "SELECT * FROM nf WHERE vector NEAR_FUSED [[1.0,0.0],[0.0,1.0]] LIMIT 10";
    let parsed = velesdb_core::velesql::Parser::parse(sql);
    assert!(
        parsed.is_ok(),
        "NEAR_FUSED with two vectors must parse: {parsed:?}"
    );
}

/// LOCK: `NEAR_FUSED []` (empty array) is a PARSE ERROR — the grammar requires
/// at least one vector (mirrors `negative_edge_tests::test_reject_near_fused_empty_array`).
/// Ground truth: `Parser::parse` returns Err for an empty NEAR_FUSED array.
#[test]
fn near_fused_empty_array_is_parse_error() {
    let sql = "SELECT * FROM nf WHERE vector NEAR_FUSED [] LIMIT 10";
    assert!(
        velesdb_core::velesql::Parser::parse(sql).is_err(),
        "empty NEAR_FUSED array must fail to parse"
    );
}

/// EXECUTES real fusion via SQL: `NEAR_FUSED [[0.8,0.6],[0.6,0.8]]` routes to
/// multi_query_search, so the diagonal id3 (closest to BOTH query vectors) is the
/// top fused result — the SAME ground truth as the engine-API test below.
#[test]
fn near_fused_via_sql_fuses_top_is_diagonal() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf");
    let results = execute_sql(
        &db,
        "SELECT * FROM nf WHERE vector NEAR_FUSED [[0.8,0.6],[0.6,0.8]] LIMIT 3",
    )
    .expect("test: NEAR_FUSED via SQL must execute");
    assert_eq!(
        results[0].point.id, 3,
        "diagonal id3 must be the top fused result"
    );
    assert_eq!(
        results.len(),
        3,
        "fusion returns up to top_k ranked results"
    );
    assert_eq!(
        result_ids(&results),
        [1u64, 2, 3].into_iter().collect(),
        "all three points retrieved across branches"
    );
}

/// The `USING FUSION 'rrf' (k=60)` clause is honored (maps to `RRF{k:60}`), not
/// ignored: id3 still tops the fused ranking.
#[test]
fn near_fused_via_sql_using_fusion_rrf_executes() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf_uf");
    let results = execute_sql(
        &db,
        "SELECT * FROM nf_uf WHERE vector NEAR_FUSED [[0.8,0.6],[0.6,0.8]] \
         USING FUSION 'rrf' (k = 60) LIMIT 3",
    )
    .expect("test: NEAR_FUSED USING FUSION must execute");
    assert_eq!(results[0].point.id, 3, "USING FUSION 'rrf' fuses to id3");
}

/// A residual metadata predicate (`AND content = 'xy'`) is threaded into
/// multi_query_search as a pre-fusion filter, so only the matching row survives.
#[test]
fn near_fused_via_sql_with_metadata_filter() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf_md");
    let results = execute_sql(
        &db,
        "SELECT * FROM nf_md WHERE vector NEAR_FUSED [[0.8,0.6],[0.6,0.8]] \
         AND content = 'xy' LIMIT 3",
    )
    .expect("test: NEAR_FUSED with metadata filter must execute");
    assert_eq!(
        result_ids(&results),
        [3u64].into_iter().collect(),
        "only id3 (content='xy') survives the pre-fusion metadata filter"
    );
}

// ============================================================================
// Surface 2 — engine-API multi-vector fusion DOES fuse (contrast)
// ============================================================================

/// CONTRAST: multi-vector fusion is reachable ONLY via the engine API
/// `VectorCollection::multi_query_search`, which DOES fuse. The queries
/// [0.8,0.6] and [0.6,0.8] both lean toward the diagonal id3=[0.7,0.7], so id3
/// is the closest point to BOTH (rank 0 in each per-query ranking) while the
/// axis points id1/id2 lead only their own query. RRF (Σ 1/(k+rank), 1-based)
/// gives id3 = 1/61+1/61 ≈ 0.03279, beating id1=id2 = 1/62+1/63 ≈ 0.03200
/// (an orthogonal [1,0]/[0,1] pair would instead let the axis points win by a
/// hair, since being rank-0 in one list beats rank-1 in both — 1/x convexity).
/// Ground truth: the top fused result id is 3.
#[test]
fn multi_query_search_fuses_top_is_diagonal() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf2");
    let vc = db
        .get_vector_collection("nf2")
        .expect("test: get collection");
    let q0: &[f32] = &[0.8, 0.6];
    let q1: &[f32] = &[0.6, 0.8];
    let results = vc
        .multi_query_search(&[q0, q1], 3, FusionStrategy::RRF { k: 60 }, None)
        .expect("test: multi_query_search");
    assert_eq!(
        results[0].point.id, 3,
        "diagonal point near both queries must be the top fused result"
    );
}

/// CONTRAST: `multi_query_search` returns a genuine ranking of size <= top_k,
/// not a full scan — and id3 (rank 0 in both diagonal-leaning branches)
/// outranks the axis points id1/id2. Ground truth: 3 results, id3 first,
/// {1,2,3} all present.
#[test]
fn multi_query_search_returns_ranking_not_scan() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf3");
    let vc = db
        .get_vector_collection("nf3")
        .expect("test: get collection");
    let q0: &[f32] = &[0.8, 0.6];
    let q1: &[f32] = &[0.6, 0.8];
    let results = vc
        .multi_query_search(&[q0, q1], 3, FusionStrategy::RRF { k: 60 }, None)
        .expect("test: multi_query_search");
    assert_eq!(
        results.len(),
        3,
        "fusion returns up to top_k ranked results"
    );
    assert_eq!(
        results[0].point.id, 3,
        "top fused result is the diagonal id3"
    );
    assert_eq!(
        result_ids(&results),
        [1u64, 2, 3].into_iter().collect(),
        "all three points are retrieved across the two branches"
    );
}

// ============================================================================
// Isolation contract — NEAR_FUSED must be the sole vector predicate
// ============================================================================

/// NEAR_FUSED under OR is rejected (not silently degraded to a metadata scan
/// that drops the fused vectors — the defect the executor cannot honor).
#[test]
fn near_fused_under_or_is_rejected() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf_or");
    let err = execute_sql(
        &db,
        "SELECT * FROM nf_or WHERE vector NEAR_FUSED [[0.8,0.6],[0.6,0.8]] OR content = 'x' LIMIT 3",
    )
    .expect_err("test: NEAR_FUSED under OR must be rejected");
    assert!(
        err.to_string()
            .contains("NEAR_FUSED must be the only vector predicate"),
        "expected isolation reject, got: {err}"
    );
}

/// NEAR mixed with NEAR_FUSED is rejected (not silently dropping the NEAR leg).
#[test]
fn near_fused_mixed_with_near_is_rejected() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf_mix");
    let err = execute_sql(
        &db,
        "SELECT * FROM nf_mix WHERE vector NEAR [1.0,0.0] \
         AND vector NEAR_FUSED [[0.8,0.6],[0.6,0.8]] LIMIT 3",
    )
    .expect_err("test: NEAR + NEAR_FUSED must be rejected");
    assert!(
        err.to_string()
            .contains("NEAR_FUSED must be the only vector predicate"),
        "expected isolation reject, got: {err}"
    );
}
