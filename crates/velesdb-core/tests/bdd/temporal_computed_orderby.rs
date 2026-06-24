//! BDD-style end-to-end tests for two VelesQL surfaces with subtle, easily
//! regressed semantics:
//!
//! 1. **Temporal boundary partitioning** — `WHERE <field> {<|>} NOW() ± INTERVAL '...'`.
//!    `NOW()` is wall-clock, so the fixtures place timestamps *far* from the
//!    boundary (year-2000 epochs vs year-3000 epochs) to keep the partition
//!    deterministic regardless of when the suite runs.
//!    (Grammar: `interval_expr = INTERVAL string`, units in
//!    `parser/values.rs::parse_interval_string`; `NOW() - INTERVAL '1 day'`
//!    evaluates to `SystemTime::now() - 86400` via
//!    `TemporalExpr::to_epoch_seconds`. The WHERE filter converts the temporal
//!    RHS to epoch seconds in `filter/conversion.rs`.)
//!
//! 2. **Computed non-monotonic ORDER BY** — `ORDER BY w1 * similarity() + w2 * <field>`.
//!    `similarity()` resolves to the cosine search score and `<field>` to the
//!    numeric payload value (`ordering.rs::ScoreContext::resolve_variable`),
//!    so a weighted blend can rank a *less* similar point above a *more*
//!    similar one. Fixtures are hand-computed so the blended order differs from
//!    both the similarity-only and the field-only orders.
//!    (Grammar: `order_by_arithmetic`, grammar.pest:296-303.)

use serde_json::json;
use std::collections::HashSet;

use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Fixture 1: temporal boundary golden set
// =========================================================================

/// Populate a "ledger" collection whose `created_at` epoch-seconds payloads
/// straddle the `NOW() - INTERVAL '1 day'` boundary with a huge margin:
///
/// | id | created_at  | calendar (UTC)       | side          |
/// |----|-------------|----------------------|---------------|
/// | 10 | 946684800   | 2000-01-01           | far past      |
/// | 11 | 978307200   | 2001-01-01           | far past      |
/// | 12 | 32503680000 | ~3000-01-01          | far future    |
/// | 13 | 33134457600 | ~3020-01-01          | far future    |
///
/// `NOW() - 86400` sits ~1.78e9 (year 2025+), i.e. firmly between the past
/// pair and the future pair, so the partition is run-time-independent.
fn setup_temporal_boundary_collection(db: &Database) {
    db.create_vector_collection("ledger", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create ledger collection");
    let vc = db
        .get_vector_collection("ledger")
        .expect("test: get ledger collection");

    vc.upsert(vec![
        Point::new(
            10,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"created_at": 946_684_800_i64})),
        ),
        Point::new(
            11,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"created_at": 978_307_200_i64})),
        ),
        Point::new(
            12,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"created_at": 32_503_680_000_i64})),
        ),
        Point::new(
            13,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"created_at": 33_134_457_600_i64})),
        ),
    ])
    .expect("test: upsert ledger corpus");
}

/// GIVEN a ledger straddling the now-boundary by decades on each side
/// WHEN `WHERE created_at > NOW() - INTERVAL '1 day'`
/// THEN exactly the two far-future ids {12, 13} are returned (the year-2000
///      ids are excluded), regardless of when the test runs.
#[test]
fn test_temporal_boundary_future_side_exact_set() {
    let (_dir, db) = create_test_db();
    setup_temporal_boundary_collection(&db);

    let sql = "SELECT * FROM ledger WHERE created_at > NOW() - INTERVAL '1 day' LIMIT 10";
    let results = execute_sql(&db, sql).expect("test: temporal future-side partition");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([12_u64, 13]),
        "only far-future created_at must exceed (now - 1 day), got {ids:?}"
    );
}

/// GIVEN the same ledger
/// WHEN `WHERE created_at < NOW() - INTERVAL '1 day'` (complement boundary)
/// THEN exactly the two far-past ids {10, 11} are returned.
#[test]
fn test_temporal_boundary_past_side_exact_set() {
    let (_dir, db) = create_test_db();
    setup_temporal_boundary_collection(&db);

    let sql = "SELECT * FROM ledger WHERE created_at < NOW() - INTERVAL '1 day' LIMIT 10";
    let results = execute_sql(&db, sql).expect("test: temporal past-side partition");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([10_u64, 11]),
        "only far-past created_at must precede (now - 1 day), got {ids:?}"
    );
}

/// GIVEN the same ledger
/// WHEN the boundary is expressed in hours — `NOW() - INTERVAL '24 hours'`
/// THEN it partitions identically to `INTERVAL '1 day'` (24h == 86400s),
///      yielding exactly the far-future ids {12, 13}. Verifies the `hours`
///      unit and unit equivalence in `parse_interval_string`.
#[test]
fn test_temporal_boundary_hours_unit_matches_day() {
    let (_dir, db) = create_test_db();
    setup_temporal_boundary_collection(&db);

    let sql = "SELECT * FROM ledger WHERE created_at > NOW() - INTERVAL '24 hours' LIMIT 10";
    let results = execute_sql(&db, sql).expect("test: temporal hours-unit partition");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([12_u64, 13]),
        "INTERVAL '24 hours' must partition like '1 day', got {ids:?}"
    );
}

// =========================================================================
// Fixture 2: computed non-monotonic ORDER BY
// =========================================================================

/// Populate a "blend" collection whose cosine similarity to query `[1,0,0,0]`
/// and whose `boost` payload pull the ranking in opposite directions:
///
/// | id | vector      | cos to [1,0,0,0] | boost |
/// |----|-------------|------------------|-------|
/// | 1  | `[1,0,0,0]` | 1.0              | 0.2   |
/// | 2  | `[3,4,0,0]` | 0.6              | 0.9   |
/// | 3  | `[4,3,0,0]` | 0.8              | 0.1   |
/// | 4  | `[1,2,0,0]` | 0.4472           | 0.7   |
///
/// Cosine = first-component / norm (query is the unit x-axis), so the integer
/// 3-4-5 / 4-3-5 triples give exact 0.6 / 0.8 cosines and `[1,2,0,0]` gives
/// 1/sqrt(5) ≈ 0.4472.
///
/// Blended key `0.5 * similarity() + 0.5 * boost`:
///   id1 = 0.500 + 0.100 = 0.600
///   id2 = 0.300 + 0.450 = 0.750
///   id3 = 0.400 + 0.050 = 0.450
///   id4 = 0.2236 + 0.350 = 0.5736
/// → DESC order [2, 1, 4, 3], which differs from BOTH single-source orders:
///   similarity-only DESC = [1, 3, 2, 4]
///   boost-only DESC      = [2, 4, 1, 3]
fn setup_blend_collection(db: &Database) {
    db.create_vector_collection("blend", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create blend collection");
    let vc = db
        .get_vector_collection("blend")
        .expect("test: get blend collection");

    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"boost": 0.2}))),
        Point::new(2, vec![3.0, 4.0, 0.0, 0.0], Some(json!({"boost": 0.9}))),
        Point::new(3, vec![4.0, 3.0, 0.0, 0.0], Some(json!({"boost": 0.1}))),
        Point::new(4, vec![1.0, 2.0, 0.0, 0.0], Some(json!({"boost": 0.7}))),
    ])
    .expect("test: upsert blend corpus");
}

/// GIVEN the blend collection
/// WHEN `ORDER BY 0.5 * similarity() + 0.5 * boost DESC`
/// THEN the blended weighting yields the exact order [2, 1, 4, 3] — id 2 (the
///      *least* similar of the top blend) wins, proving the computed key is
///      applied rather than the raw similarity.
#[test]
fn test_computed_order_by_blend_desc_exact_order() {
    let (_dir, db) = create_test_db();
    setup_blend_collection(&db);

    let sql = "SELECT * FROM blend WHERE vector NEAR $v \
               ORDER BY 0.5 * similarity() + 0.5 * boost DESC LIMIT 10";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: computed blend ORDER BY DESC");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![2, 1, 4, 3],
        "blended DESC key must rank by 0.5*sim+0.5*boost, got {ids:?}"
    );
}

/// GIVEN the blend collection
/// WHEN the same blended key is sorted ASC
/// THEN the order is the exact reverse: [3, 4, 1, 2].
#[test]
fn test_computed_order_by_blend_asc_exact_order() {
    let (_dir, db) = create_test_db();
    setup_blend_collection(&db);

    let sql = "SELECT * FROM blend WHERE vector NEAR $v \
               ORDER BY 0.5 * similarity() + 0.5 * boost ASC LIMIT 10";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: computed blend ORDER BY ASC");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![3, 4, 1, 2],
        "blended ASC key must be the exact reverse of DESC, got {ids:?}"
    );
}

/// GIVEN the blend collection
/// WHEN the blended computed ORDER BY is compared to a pure
///      `ORDER BY similarity() DESC` baseline over the identical query
/// THEN the two orderings differ — the blend is genuinely non-monotonic in
///      similarity (blend = [2,1,4,3] vs similarity-only = [1,3,2,4]).
#[test]
fn test_computed_order_by_differs_from_similarity_only() {
    let (_dir, db) = create_test_db();
    setup_blend_collection(&db);
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);

    let similarity_only = execute_sql_with_params(
        &db,
        "SELECT * FROM blend WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 10",
        &params,
    )
    .expect("test: similarity-only baseline");
    let sim_ids: Vec<u64> = similarity_only.iter().map(|r| r.point.id).collect();
    assert_eq!(
        sim_ids,
        vec![1, 3, 2, 4],
        "similarity-only DESC baseline must rank by cosine, got {sim_ids:?}"
    );

    let blended = execute_sql_with_params(
        &db,
        "SELECT * FROM blend WHERE vector NEAR $v \
         ORDER BY 0.5 * similarity() + 0.5 * boost DESC LIMIT 10",
        &params,
    )
    .expect("test: blended computed ORDER BY");
    let blend_ids: Vec<u64> = blended.iter().map(|r| r.point.id).collect();

    assert_ne!(
        blend_ids, sim_ids,
        "blended order must diverge from similarity-only order"
    );
    assert_eq!(
        blend_ids,
        vec![2, 1, 4, 3],
        "blended order must be the hand-computed [2,1,4,3], got {blend_ids:?}"
    );
}
