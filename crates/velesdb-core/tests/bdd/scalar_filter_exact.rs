//! BDD-style end-to-end tests pinning small, EXACT scalar-filter positives that
//! the rest of the suite under-covers: `IS NULL` / `IS NOT NULL` complementary
//! id sets, a `WHERE similarity(vector, $v) >= threshold` threshold partition,
//! and a `WITH (ef_search = N)` query-time override returning the right rows.
//!
//! Each scenario follows GIVEN (setup data) -> WHEN (execute SQL) -> THEN
//! (verify exact ids), exercising the full pipeline:
//! SQL string -> `Parser::parse()` -> `Database::execute_query()` -> verify.
//!
//! ## Verified against source
//!
//! - `IS [NOT] NULL` grammar: `grammar.pest:469`
//!   (`is_null_expr = { where_column ~ ^"IS" ~ not_kw? ~ ^"NULL" }`); the AST
//!   maps to `Condition::IsNull` / `IsNotNull` in `filter/conversion.rs:152`.
//! - `similarity(field, vector) op threshold` grammar: `grammar.pest:395`.
//!   A `Condition::Similarity` is itself score-producing
//!   (`validation.rs:361 has_score_producing_condition`), so it is **valid in a
//!   bare WHERE** (NOT V006-rejected — V006 only fires for `similarity()` in
//!   SELECT/ORDER BY without a NEAR/similarity context). Execution recomputes
//!   the metric score against the point's own vector when `field == "vector"`
//!   and keeps rows passing the threshold (`similarity_filter.rs:36`,
//!   `where_eval.rs:253`); Cosine is `higher_is_better`, so `>=` keeps
//!   `score >= threshold` (`distance.rs:155`).
//! - `WITH (ef_search = N)` grammar: `grammar.pest:306` (SELECT suffix at
//!   `grammar.pest:221`, after LIMIT); see `tests/epic_features_integration_tests.rs:423`.

use serde_json::json;
use velesdb_core::{Database, DistanceMetric, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `tagged` collection where the `tag` field is present (string),
/// absent (omitted), or JSON null, so `IS NULL` / `IS NOT NULL` partition the
/// rows into known, complementary id sets.
///
/// | id | tag      | tag state          |
/// |----|----------|--------------------|
/// | 1  | "red"    | present (non-null) |
/// | 2  | "blue"   | present (non-null) |
/// | 3  | null     | JSON null          |
/// | 4  | (absent) | field omitted      |
/// | 5  | "green"  | present (non-null) |
///
/// `IS NULL` matches the JSON-null AND the absent field -> {3, 4}.
/// `IS NOT NULL` matches the present-string rows -> {1, 2, 5}.
fn setup_tagged(db: &Database) {
    db.create_vector_collection("tagged", 4, DistanceMetric::Cosine)
        .expect("test: create tagged collection");
    let vc = db
        .get_vector_collection("tagged")
        .expect("test: get tagged collection");

    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"tag": "red"}))),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"tag": "blue"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"tag": null}))),
        Point::new(4, vec![0.0, 0.0, 0.0, 1.0], Some(json!({"other": "x"}))),
        Point::new(5, vec![0.5, 0.5, 0.0, 0.0], Some(json!({"tag": "green"}))),
    ])
    .expect("test: upsert tagged");
}

/// Build a `sims` collection (dim 4, Cosine) of well-separated points along
/// `[1, off, 0, 0]` plus one orthogonal point. With query `[1, 0, 0, 0]`,
/// cosine = `1 / sqrt(1 + off^2)`, strictly decreasing in `off`:
///
/// | id | vector            | off | cosine similarity |
/// |----|-------------------|-----|-------------------|
/// | 10 | `[1.0, 0.0, 0, 0]`| 0.0 | 1.0000            |
/// | 11 | `[1.0, 0.3, 0, 0]`| 0.3 | 0.9578            |
/// | 12 | `[1.0, 0.7, 0, 0]`| 0.7 | 0.8192            |
/// | 13 | `[1.0, 1.2, 0, 0]`| 1.2 | 0.6402            |
/// | 14 | `[1.0, 3.0, 0, 0]`| 3.0 | 0.3162            |
/// | 15 | `[0.0, 1.0, 0, 0]`|  -  | 0.0000 (orthogonal) |
///
/// A `>= 0.7` threshold cleanly partitions the set (wide gap 0.819 vs 0.640):
/// pass = {10, 11, 12}; fail = {13, 14, 15}.
fn setup_sims(db: &Database) {
    db.create_vector_collection("sims", 4, DistanceMetric::Cosine)
        .expect("test: create sims collection");
    let vc = db
        .get_vector_collection("sims")
        .expect("test: get sims collection");

    vc.upsert(vec![
        Point::new(10, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"n": 10}))),
        Point::new(11, vec![1.0, 0.3, 0.0, 0.0], Some(json!({"n": 11}))),
        Point::new(12, vec![1.0, 0.7, 0.0, 0.0], Some(json!({"n": 12}))),
        Point::new(13, vec![1.0, 1.2, 0.0, 0.0], Some(json!({"n": 13}))),
        Point::new(14, vec![1.0, 3.0, 0.0, 0.0], Some(json!({"n": 14}))),
        Point::new(15, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"n": 15}))),
    ])
    .expect("test: upsert sims");
}

// =========================================================================
// Scenario 1: IS NULL matches JSON-null AND absent fields
// =========================================================================

/// GIVEN `tagged` (rows 3=null, 4=absent, others present)
/// WHEN `WHERE tag IS NULL`
/// THEN the EXACT id set is {3, 4} — both the JSON-null and the omitted field.
#[test]
fn test_is_null_matches_null_and_absent() {
    let (_dir, db) = create_test_db();
    setup_tagged(&db);

    let results = execute_sql(&db, "SELECT * FROM tagged WHERE tag IS NULL LIMIT 10;")
        .expect("test: IS NULL filter should succeed");

    let ids = result_ids(&results);
    let expected: std::collections::HashSet<u64> = [3, 4].into_iter().collect();
    assert_eq!(
        ids, expected,
        "IS NULL must match the JSON-null (id=3) and the absent-field (id=4) rows"
    );
}

// =========================================================================
// Scenario 2: IS NOT NULL is the exact complement
// =========================================================================

/// GIVEN the same `tagged` collection
/// WHEN `WHERE tag IS NOT NULL`
/// THEN the EXACT id set is {1, 2, 5} — the complement of the IS NULL set,
///      i.e. every row whose `tag` is a present, non-null value.
#[test]
fn test_is_not_null_is_exact_complement() {
    let (_dir, db) = create_test_db();
    setup_tagged(&db);

    let results = execute_sql(&db, "SELECT * FROM tagged WHERE tag IS NOT NULL LIMIT 10;")
        .expect("test: IS NOT NULL filter should succeed");

    let ids = result_ids(&results);
    let expected: std::collections::HashSet<u64> = [1, 2, 5].into_iter().collect();
    assert_eq!(
        ids, expected,
        "IS NOT NULL must match exactly the present-value rows (1, 2, 5)"
    );
}

// =========================================================================
// Scenario 3: IS NULL + IS NOT NULL together cover the whole collection
// =========================================================================

/// GIVEN the same `tagged` collection (5 rows)
/// WHEN both `IS NULL` and `IS NOT NULL` are run
/// THEN the two id sets are DISJOINT and their union is all 5 ids — proving the
///      predicates are exact complements over the collection.
#[test]
fn test_is_null_and_not_null_partition_collection() {
    let (_dir, db) = create_test_db();
    setup_tagged(&db);

    let nulls = result_ids(
        &execute_sql(&db, "SELECT * FROM tagged WHERE tag IS NULL LIMIT 10;")
            .expect("test: IS NULL"),
    );
    let non_nulls = result_ids(
        &execute_sql(&db, "SELECT * FROM tagged WHERE tag IS NOT NULL LIMIT 10;")
            .expect("test: IS NOT NULL"),
    );

    assert!(
        nulls.is_disjoint(&non_nulls),
        "IS NULL and IS NOT NULL must not overlap, got {nulls:?} vs {non_nulls:?}"
    );
    let union: std::collections::HashSet<u64> = nulls.union(&non_nulls).copied().collect();
    let all: std::collections::HashSet<u64> = [1, 2, 3, 4, 5].into_iter().collect();
    assert_eq!(
        union, all,
        "the two predicates must cover every row exactly once"
    );
}

// =========================================================================
// Scenario 4: similarity(vector, $v) >= threshold selects the exact set
// =========================================================================

/// GIVEN the `sims` cosine collection
/// WHEN `WHERE similarity(vector, $v) >= 0.7` with $v = [1,0,0,0]
/// THEN the EXACT id set is {10, 11, 12} — the rows whose cosine to $v is
///      >= 0.7 (1.0, 0.958, 0.819); the rest (0.640, 0.316, 0.0) fall below.
///      The 0.819 vs 0.640 gap is wide, so the f32 threshold is unambiguous.
#[test]
fn test_similarity_threshold_selects_exact_set() {
    let (_dir, db) = create_test_db();
    setup_sims(&db);

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM sims WHERE similarity(vector, $v) >= 0.7 LIMIT 10;",
        &vector_param(&[1.0, 0.0, 0.0, 0.0]),
    )
    .expect("test: similarity threshold filter should succeed");

    let ids = result_ids(&results);
    let expected: std::collections::HashSet<u64> = [10, 11, 12].into_iter().collect();
    assert_eq!(
        ids, expected,
        "similarity >= 0.7 must keep exactly the cosine->=0.7 rows (10, 11, 12)"
    );
}

// =========================================================================
// Scenario 5: a higher similarity threshold tightens the set
// =========================================================================

/// GIVEN the same `sims` collection
/// WHEN `WHERE similarity(vector, $v) >= 0.9` with $v = [1,0,0,0]
/// THEN the EXACT id set is {10, 11} — only cosine 1.0 and 0.958 clear 0.9;
///      id 12 (0.819) now falls below. Confirms the threshold is honored, not
///      merely "any positive similarity".
#[test]
fn test_similarity_higher_threshold_tightens_set() {
    let (_dir, db) = create_test_db();
    setup_sims(&db);

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM sims WHERE similarity(vector, $v) >= 0.9 LIMIT 10;",
        &vector_param(&[1.0, 0.0, 0.0, 0.0]),
    )
    .expect("test: tight similarity threshold should succeed");

    let ids = result_ids(&results);
    let expected: std::collections::HashSet<u64> = [10, 11].into_iter().collect();
    assert_eq!(
        ids, expected,
        "similarity >= 0.9 must keep only cosine-1.0 (10) and 0.958 (11)"
    );
}

// =========================================================================
// Scenario 6: WITH (ef_search = N) parses, executes, returns the right rows
// =========================================================================

/// GIVEN the `sims` cosine collection (well separated along one axis)
/// WHEN `vector NEAR $v LIMIT 3 WITH (ef_search = 64)` with $v = [1,0,0,0]
/// THEN the clause parses and executes, returning the EXACT top-3 nearest ids
///      in descending-cosine order [10, 11, 12]. (We assert result-set
///      correctness, not the internal ef value, which is not user-observable.)
#[test]
fn test_with_ef_search_returns_expected_top3() {
    let (_dir, db) = create_test_db();
    setup_sims(&db);

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM sims WHERE vector NEAR $v LIMIT 3 WITH (ef_search = 64);",
        &vector_param(&[1.0, 0.0, 0.0, 0.0]),
    )
    .expect("test: WITH (ef_search = 64) should parse and execute");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![10, 11, 12],
        "WITH (ef_search) NEAR LIMIT 3 must return the exact descending-cosine top-3"
    );
}

// =========================================================================
// Scenario 7: WITH (ef_search = N) is consistent with the no-override query
// =========================================================================

/// GIVEN the same `sims` collection
/// WHEN the same `vector NEAR $v LIMIT 4` is run with and without
///      `WITH (ef_search = 128)`
/// THEN both return the IDENTICAL exact ordered top-4 [10, 11, 12, 13],
///      proving the ef override leaves the result set unchanged on this
///      cleanly-separated dataset (the override only affects recall on harder
///      data, which is not what this scenario pins).
#[test]
fn test_with_ef_search_matches_baseline() {
    let (_dir, db) = create_test_db();
    setup_sims(&db);

    let v = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let baseline =
        execute_sql_with_params(&db, "SELECT * FROM sims WHERE vector NEAR $v LIMIT 4;", &v)
            .expect("test: baseline NEAR should succeed");
    let with_ef = execute_sql_with_params(
        &db,
        "SELECT * FROM sims WHERE vector NEAR $v LIMIT 4 WITH (ef_search = 128);",
        &v,
    )
    .expect("test: NEAR with ef_search should succeed");

    let baseline_ids: Vec<u64> = baseline.iter().map(|r| r.point.id).collect();
    let with_ef_ids: Vec<u64> = with_ef.iter().map(|r| r.point.id).collect();
    assert_eq!(
        baseline_ids,
        vec![10, 11, 12, 13],
        "baseline top-4 must be the exact cosine order"
    );
    assert_eq!(
        with_ef_ids, baseline_ids,
        "ef_search override must not change the result set on well-separated data"
    );
}
