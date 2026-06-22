//! BDD tests for the two `NEAR_FUSED` surfaces: the rejected SQL surface and
//! the engine-level multi-vector fusion API.
//!
//! Contract under test (two surfaces):
//!
//! 1. **VelesQL `NEAR_FUSED` parses but is REJECTED at validation (V012).** The
//!    grammar (`grammar.pest` `vector_fused_search`) and parser
//!    (`condition_vectors.rs::parse_vector_fused_search`) accept
//!    `vector NEAR_FUSED [[..],[..]] [USING FUSION ...]` and build a
//!    `Condition::VectorFusedSearch`, but it has NO executor: left unchecked it
//!    would silently degrade to an unranked full scan
//!    (`extraction.rs::extract_vector_search` extracts no query vector;
//!    `where_eval.rs` treats `VectorFusedSearch(_)` as always-true). Rather than
//!    return wrong rows, `validation.rs::reject_near_fused` rejects it with V012.
//!    The full reject contract lives in `velesql_reject_conformance.rs`.
//!
//! 2. **Multi-vector fusion is engine-API-only.** The fusion that `NEAR_FUSED`
//!    *looks* like it should perform is reachable solely through
//!    `VectorCollection::multi_query_search`, which DOES fuse.

use velesdb_core::{Database, FusionStrategy, Point};

use super::helpers::{create_test_db, result_ids};

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
// Surface 1 — VelesQL NEAR_FUSED parse-only no-op (regression lock)
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

/// REJECT (V012): a `NEAR_FUSED` query run through the SQL pipeline is now
/// rejected at validation rather than silently degrading to an unranked full
/// scan (which would return wrong rows). Ground truth: `execute_sql` returns
/// Err whose message carries the V012 marker. The full reject contract — incl.
/// the USING FUSION variant — lives in `velesql_reject_conformance.rs`.
#[test]
fn near_fused_via_sql_is_rejected_v012() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf");
    let err = super::helpers::execute_sql(
        &db,
        "SELECT * FROM nf WHERE vector NEAR_FUSED [[1.0,0.0],[0.0,1.0]] LIMIT 10",
    )
    .expect_err("test: NEAR_FUSED via SQL must be rejected, not a no-op scan");
    assert!(
        err.to_string().contains("V012"),
        "expected V012 NearFusedNotExecutable, got: {err}"
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
