//! BDD tests locking the `NEAR_FUSED` parse-only no-op and contrasting it with
//! the engine-level multi-vector fusion API.
//!
//! Contract under test (two surfaces):
//!
//! 1. **VelesQL `NEAR_FUSED` is parse-only.** The grammar
//!    (`grammar.pest` `vector_fused_search`) and parser
//!    (`condition_vectors.rs::parse_vector_fused_search`) accept
//!    `vector NEAR_FUSED [[..],[..]] [USING FUSION ...]` and build a
//!    `Condition::VectorFusedSearch`, but execution NEVER fuses on it:
//!    - `extraction.rs::extract_vector_search` has no `VectorFusedSearch` arm
//!      (`_ => Ok(None)`), so no query vector is ever extracted from it.
//!    - `where_eval.rs:214` treats `VectorFusedSearch(_) => Ok(true)`, i.e. an
//!      always-true predicate.
//!    The net effect: a `SELECT ... WHERE vector NEAR_FUSED [...]` is an
//!    unranked full scan returning every row, NOT a fusion ranking. These tests
//!    LOCK that behavior; if `NEAR_FUSED` is ever wired to real fusion they must
//!    be updated.
//!
//! 2. **Multi-vector fusion is engine-API-only.** The same fusion that
//!    `NEAR_FUSED` *looks* like it should perform is reachable solely through
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

/// LOCK (the core no-op): a two-vector `NEAR_FUSED` query is an UNRANKED FULL
/// SCAN, not a fusion ranking — it returns every stored row because
/// `where_eval.rs:214` evaluates `VectorFusedSearch(_) => Ok(true)` and
/// `extraction.rs` extracts no query vector from it. Ground truth: with 3
/// stored points {1,2,3}, the result id-set is exactly {1,2,3}.
/// CORRECT behavior would be a fused ranking over the two query vectors;
/// update this test if `NEAR_FUSED` is ever wired to real fusion.
#[test]
fn near_fused_returns_unranked_full_scan() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf");
    let results = super::helpers::execute_sql(
        &db,
        "SELECT * FROM nf WHERE vector NEAR_FUSED [[1.0,0.0],[0.0,1.0]] LIMIT 10",
    )
    .expect("test: execute NEAR_FUSED query");
    assert_eq!(
        result_ids(&results),
        [1u64, 2, 3].into_iter().collect(),
        "NEAR_FUSED is parse-only: must return all rows as an unranked scan"
    );
}

/// LOCK: the parse-only no-op is independent of the `USING FUSION` clause —
/// supplying `USING FUSION 'rrf'` changes nothing; it is still an unranked
/// full scan over all rows. Ground truth: result id-set is exactly {1,2,3}.
#[test]
fn near_fused_with_using_fusion_clause_still_no_op() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf_using");
    let results = super::helpers::execute_sql(
        &db,
        "SELECT * FROM nf_using WHERE vector NEAR_FUSED [[1.0,0.0],[0.0,1.0]] \
         USING FUSION 'rrf' LIMIT 10",
    )
    .expect("test: execute NEAR_FUSED USING FUSION query");
    assert_eq!(
        result_ids(&results),
        [1u64, 2, 3].into_iter().collect(),
        "USING FUSION does not activate fusion: NEAR_FUSED remains a no-op scan"
    );
}

/// LOCK: because `NEAR_FUSED` contributes no ranking, the query is sensitive
/// ONLY to the scan's row set and `LIMIT`, never to the query vectors. A
/// completely different pair of query vectors yields the SAME id-set as the
/// scan above. Ground truth: result id-set is exactly {1,2,3}.
#[test]
fn near_fused_ignores_query_vectors() {
    let (_dir, db) = create_test_db();
    setup_nf(&db, "nf_ignore");
    let results = super::helpers::execute_sql(
        &db,
        "SELECT * FROM nf_ignore WHERE vector NEAR_FUSED [[0.123,0.999],[0.5,0.5]] LIMIT 10",
    )
    .expect("test: execute NEAR_FUSED with unrelated vectors");
    assert_eq!(
        result_ids(&results),
        [1u64, 2, 3].into_iter().collect(),
        "NEAR_FUSED ranking is a no-op: arbitrary query vectors return the same scan"
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
