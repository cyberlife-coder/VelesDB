//! BDD integration tests for FUSION / similarity() / EXPLAIN
//! in the WASM VelesQL executor (S4-13).

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

fn db_with_vectors() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    for (id, v, cat) in [
        (1u64, "[1.0, 0.0, 0.0, 0.0]", "a"),
        (2, "[0.9, 0.1, 0.0, 0.0]", "a"),
        (3, "[0.0, 1.0, 0.0, 0.0]", "b"),
        (4, "[0.0, 0.0, 1.0, 0.0]", "b"),
    ] {
        execute(
            &mut db,
            &format!("INSERT INTO vecs (id, vector, cat) VALUES ({id}, $v, '{cat}')"),
            Some(&format!("{{\"v\": {v}}}")),
        )
        .expect("test: seed");
    }
    db
}

// =========================================================================
// similarity() threshold — nominal
// =========================================================================

#[test]
fn test_similarity_threshold_filters_low_scores() {
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5 LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: similarity");
    // Only id=1 (1.0) and id=2 (~0.9939) should pass the >0.5 threshold.
    assert!(r.row_count() >= 2);
    assert!(r.row_count() <= 4);
}

#[test]
fn test_similarity_combined_with_payload_filter() {
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5 AND cat = 'b' LIMIT 10",
        Some(r#"{"q": [0.0, 1.0, 0.0, 0.0]}"#),
    )
    .expect("test: similarity + filter");
    // cat='b' has ids 3 and 4; only 3 passes the > 0.5 threshold (sim=1.0).
    assert_eq!(r.row_count(), 1);
    assert_eq!(r.row(0).expect("test: row").id(), 3);
}

// =========================================================================
// FUSION — nominal
// =========================================================================

#[test]
fn test_fusion_rrf_returns_ranked_results() {
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q AND cat = 'a' LIMIT 10 USING FUSION (strategy = 'rrf')",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: rrf fusion");
    // Both branches return ids; FUSION is tolerant and never errors.
    assert!(r.row_count() >= 1);
}

// =========================================================================
// EXPLAIN — nominal
// =========================================================================

#[test]
fn test_explain_select_returns_plan_rows() {
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "EXPLAIN SELECT * FROM vecs WHERE cat = 'a' LIMIT 10",
        None,
    )
    .expect("test: explain");
    assert!(r.row_count() >= 2);
    assert!(r.rows_json().contains("Scan"));
}

#[test]
fn test_explain_with_group_by_has_groupby_step() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("t").expect("test: t");
    execute(
        &mut db,
        "INSERT INTO t (id, c) VALUES (1, 'x'), (2, 'y')",
        None,
    )
    .expect("test: seed");
    let r = execute(
        &mut db,
        "EXPLAIN SELECT c, COUNT(*) FROM t GROUP BY c",
        None,
    )
    .expect("test: explain gb");
    assert!(r.rows_json().contains("GroupBy"));
}

// =========================================================================
// CREATE/DROP INDEX + ANALYZE no-op — nominal
// =========================================================================

#[test]
fn test_create_index_noop_returns_ddl_result() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    let r = execute(&mut db, "CREATE INDEX ON docs (category)", None).expect("test: idx");
    assert_eq!(r.kind(), "ddl");
    assert!(r.rows_json().contains("accepted-noop"));
}

#[test]
fn test_drop_index_noop_returns_ddl_result() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    let r = execute(&mut db, "DROP INDEX ON docs (category)", None).expect("test: drop idx");
    assert_eq!(r.kind(), "ddl");
}

#[test]
fn test_analyze_returns_synthetic_stats() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    execute(&mut db, "INSERT INTO docs (id) VALUES (1), (2), (3)", None).expect("test: seed");
    let r = execute(&mut db, "ANALYZE docs", None).expect("test: analyze");
    assert_eq!(r.kind(), "ddl");
    assert!(r.rows_json().contains("\"row_count\":3"));
}

// =========================================================================
// Edge cases
// =========================================================================

#[test]
fn test_similarity_on_metadata_collection_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("m").expect("test: m");
    let err = execute(
        &mut db,
        "SELECT * FROM m WHERE similarity(vector, $q) > 0.5",
        Some(r#"{"q": [1.0, 0.0]}"#),
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("metadata-only"));
}

// =========================================================================
// Negative (≥ 20%)
// =========================================================================

#[test]
fn test_similarity_dim_mismatch_errors() {
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5 LIMIT 10",
        Some(r#"{"q": [1.0, 0.0]}"#),
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("dimension mismatch"));
}

#[test]
fn test_explain_missing_collection_surfaces_scan_step() {
    let mut db = DatabaseInner::new();
    // EXPLAIN on a ghost collection: plan builder uses 0 rows hint, no error.
    let r = execute(&mut db, "EXPLAIN SELECT * FROM ghost LIMIT 10", None)
        .expect("test: explain ghost");
    assert!(r.rows_json().contains("Scan"));
}

#[test]
fn test_analyze_missing_collection_errors() {
    let mut db = DatabaseInner::new();
    let err = execute(&mut db, "ANALYZE ghost", None);
    assert!(err.is_err());
}

#[test]
fn test_similarity_unbound_param_errors() {
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $missing) > 0.5 LIMIT 10",
        Some("{}"),
    );
    assert!(err.is_err());
}

// =========================================================================
// NOT similarity — polarity preservation (finding E)
// =========================================================================

#[test]
fn test_similarity_not_greater_than_becomes_lte() {
    // `NOT sim > 0.5` must behave like `sim <= 0.5`: keeps the rows with
    // a low score, drops the high-score ones. Here id=1 (1.0) and id=2
    // (~0.9939) are above 0.5; id=3 and id=4 are at 0.0.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE NOT similarity(vector, $q) > 0.5 LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: NOT sim > 0.5");
    // Complement of the >0.5 set: ids 3 and 4.
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&3));
    assert!(ids.contains(&4));
}

#[test]
fn test_similarity_not_less_than_becomes_gte() {
    // `NOT sim < 0.5` → `sim >= 0.5`: keeps ids 1 and 2.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE NOT similarity(vector, $q) < 0.5 LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: NOT sim < 0.5");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
}

#[test]
fn test_similarity_not_equal_becomes_neq() {
    // `NOT sim = 1.0` → `sim != 1.0`: only id=1 has exact cosine 1.0.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE NOT similarity(vector, $q) = 1.0 LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: NOT sim = 1.0");
    // All ids except id=1 (which has score == 1.0).
    assert_eq!(r.row_count(), 3);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(!ids.contains(&1));
}

#[test]
fn test_similarity_plain_without_not_is_unchanged() {
    // Non-regression: plain `sim > 0.5` still behaves as before.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5 LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: plain sim > 0.5");
    // ids 1 and 2 are > 0.5.
    assert_eq!(r.row_count(), 2);
}

// =========================================================================
// De Morgan over compound NOT (finding F) — end-to-end semantics
// =========================================================================
//
// These tests pin the De Morgan rewrite applied by
// `velesql_logic::push_not_inward`. They all use the 4-row fixture
// from `db_with_vectors()`:
//
// | id | vec                  | cat |  sim vs [1,0,0,0] |
// |----|----------------------|-----|-------------------|
// |  1 | [1.0, 0, 0, 0]       | 'a' |   1.0             |
// |  2 | [0.9, 0.1, 0, 0]     | 'a' |   ~0.9939         |
// |  3 | [0, 1.0, 0, 0]       | 'b' |   0.0             |
// |  4 | [0, 0, 1.0, 0]       | 'b' |   0.0             |

#[test]
fn test_not_compound_similarity_and_predicate_is_demorgan_distributed() {
    // `NOT (sim > 0.5 AND cat = 'a')` must distribute to
    // `sim <= 0.5 OR cat != 'a'` — the pre-fix implementation kept the
    // un-flipped similarity in the extractor AND left `NOT (cat='a')`
    // as residual, producing `sim > 0.5 AND cat != 'a'` (= row 2 only).
    //
    // Correct answer: rows where (sim <= 0.5) OR (cat != 'a')
    //   id 1: sim=1.0 (no) OR cat='a' (no) → false
    //   id 2: sim≈0.99 (no) OR cat='a' (no) → false
    //   id 3: sim=0.0 (yes) → true
    //   id 4: sim=0.0 (yes) → true
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE NOT (similarity(vector, $q) > 0.5 AND cat = 'a') LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: NOT (sim AND pred)");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&3));
    assert!(ids.contains(&4));
}

#[test]
fn test_not_compound_similarity_or_predicate_is_demorgan_distributed() {
    // `NOT (sim > 0.5 OR cat = 'b')` must distribute to
    // `sim <= 0.5 AND cat != 'b'`.
    //   id 1: sim=1.0 → sim<=0.5 false → false
    //   id 2: sim≈0.99 → sim<=0.5 false → false
    //   id 3: cat='b' → cat!='b' false → false
    //   id 4: cat='b' → cat!='b' false → false
    // Expected: 0 rows.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE NOT (similarity(vector, $q) > 0.5 OR cat = 'b') LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: NOT (sim OR pred)");
    assert_eq!(r.row_count(), 0);
}

#[test]
fn test_not_double_negation_simplifies() {
    // `NOT NOT (sim > 0.5)` must collapse to `sim > 0.5`, keeping ids 1 and 2.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE NOT (NOT similarity(vector, $q) > 0.5) LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: NOT NOT sim > 0.5");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
}

#[test]
fn test_not_nested_compound_with_similarity() {
    // `NOT (cat = 'a' OR (id = 3 AND sim > 0.5))`
    //   → `cat != 'a' AND (id != 3 OR sim <= 0.5)`
    //   id 1: cat='a' → cat!='a' false → false
    //   id 2: cat='a' → cat!='a' false → false
    //   id 3: cat!='a' (true) AND (id!=3 (false) OR sim<=0.5 (true, sim=0))
    //         → true AND (false OR true) → true
    //   id 4: cat!='a' (true) AND (id!=3 (true) OR sim<=0.5 (true))
    //         → true AND true → true
    // Expected: ids 3 and 4.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE NOT (cat = 'a' OR (id = 3 AND similarity(vector, $q) > 0.5)) LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: deep nested NOT");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&3));
    assert!(ids.contains(&4));
}

#[test]
fn test_not_simple_predicate_without_similarity_still_correct() {
    // Non-regression: `NOT (cat = 'a' AND id = 1)` on a metadata-only
    // collection (no similarity at all) must still produce the correct
    // De Morgan expansion: `cat != 'a' OR id != 1`.
    //
    // Fixture: 3 rows with cats 'a', 'a', 'b' at ids 1, 2, 3.
    //   id 1 cat='a': cat!='a' false OR id!=1 false → false
    //   id 2 cat='a': cat!='a' false OR id!=1 true  → true
    //   id 3 cat='b': cat!='b' (true, since b!=a) OR id!=1 true → true
    // Expected: ids 2 and 3.
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("m").expect("test: create");
    execute(
        &mut db,
        "INSERT INTO m (id, cat) VALUES (1, 'a'), (2, 'a'), (3, 'b')",
        None,
    )
    .expect("test: seed");
    let r = execute(
        &mut db,
        "SELECT * FROM m WHERE NOT (cat = 'a' AND id = 1) LIMIT 10",
        None,
    )
    .expect("test: NOT (cat AND id)");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
}

// =========================================================================
// OR(stripped-leaf, predicate) collapses to "no post-filter" (finding G)
// =========================================================================
//
// These tests pin the semantics of `strip_condition_if` under `OR`. A
// branch stripped out is logically `true` (handled externally by the
// vector / similarity path). Therefore:
//   - `true AND x` = `x`         → residual is `x`
//   - `true OR x`  = `true`      → residual is None (no post-filter)
//
// The pre-fix behaviour collapsed `OR(None, Some(x))` to `Some(x)`, which
// turned "all rows (from NEAR) OR cat = 'a'" into a post-filter of just
// "cat = 'a'" — wrongly dropping rows that didn't match the predicate,
// even though they already passed the implicit vector branch.

#[test]
fn test_or_near_with_predicate_does_not_filter_non_matching_rows() {
    // `vector NEAR $q OR cat = 'a'`: semantically the NEAR branch matches
    // every row (with a score), so the OR is trivially satisfied. The
    // residual must not post-filter on `cat = 'a'`. All 4 rows survive.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q OR cat = 'a' LIMIT 4",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: OR near");
    assert_eq!(r.row_count(), 4, "OR branch must not drop non-'a' rows");
}

#[test]
fn test_or_similarity_with_predicate_does_not_filter_out_rows_below_threshold() {
    // `similarity(v, $q) > 0.5 OR cat = 'a'`:
    //   id 1: sim=1.0 (true) → true
    //   id 2: sim≈0.99 (true) → true
    //   id 3: cat='b', sim=0 → false OR false → false
    //   id 4: cat='b', sim=0 → false OR false → false
    // Expected: ids 1 and 2 (both match via similarity or cat=a).
    // This exercises evaluate_where_with_similarity's OR arm, not the
    // strip path — but it guards against a regression should the
    // similarity branch ever route through strip_similarity's residual.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5 OR cat = 'a' LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: OR similarity");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
}

#[test]
fn test_and_near_with_predicate_still_filters_correctly() {
    // Non-regression: `vector NEAR $q AND cat = 'a' LIMIT 4` must still
    // restrict results to rows with cat='a' (ids 1 and 2). `true AND x`
    // collapses to `x`, so the residual is the cat predicate.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q AND cat = 'a' LIMIT 4",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: AND near");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
}

#[test]
fn test_and_similarity_with_predicate_still_filters_correctly() {
    // Non-regression: `similarity(v, $q) > 0.5 AND cat = 'a'` must still
    // restrict to rows matching both predicates. ids 1 and 2 pass.
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5 AND cat = 'a' LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: AND similarity");
    assert_eq!(r.row_count(), 2);
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
}

// =========================================================================
// Multiple similarity() conditions (finding H)
// =========================================================================
//
// `SimilarityEvaluator` pre-computes scores for ONE query vector. When
// the WHERE clause contains two `similarity()` predicates referencing
// DIFFERENT vectors, the evaluator silently reuses the first vector's
// scores for both thresholds — returning wrong rows. We fail loud
// instead (option 2: explicit error, no silent wrong answers).

#[test]
fn test_multiple_similarity_same_vector_is_accepted() {
    // Two similarity() predicates that reference the SAME param `$q`
    // must be accepted. Range predicate: keep rows with 0.5 < sim < 0.999.
    // id 1: sim=1.0 → NOT (sim < 0.999) → dropped
    // id 2: sim≈0.9939 → 0.5 < 0.9939 < 0.999 → kept
    // id 3 / id 4: sim=0 → NOT (sim > 0.5) → dropped
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5 AND similarity(vector, $q) < 0.999 LIMIT 10",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: same-vector similarity range");
    assert_eq!(r.row_count(), 1);
    assert_eq!(r.row(0).expect("test: row").id(), 2);
}

#[test]
fn test_multiple_similarity_different_params_returns_error_and() {
    // Two similarity() predicates referencing DIFFERENT params must be
    // rejected with a clear error rather than silently scored against
    // only `$q1`.
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q1) > 0.5 AND similarity(vector, $q2) > 0.3 LIMIT 10",
        Some(r#"{"q1": [1.0, 0.0, 0.0, 0.0], "q2": [0.0, 1.0, 0.0, 0.0]}"#),
    );
    assert!(err.is_err(), "multi-vector similarity must fail loud");
    let msg = err.expect_err("test: err");
    assert!(
        msg.contains("Multiple similarity()"),
        "error should name the feature, got: {msg}"
    );
}

#[test]
fn test_multiple_similarity_different_params_returns_error_or() {
    // Same check under OR composition: the evaluator still pre-computes
    // for one vector only, so we must reject.
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, $q1) > 0.5 OR similarity(vector, $q2) > 0.5 LIMIT 10",
        Some(r#"{"q1": [1.0, 0.0, 0.0, 0.0], "q2": [0.0, 1.0, 0.0, 0.0]}"#),
    );
    assert!(err.is_err());
    assert!(err
        .expect_err("test: err")
        .contains("Multiple similarity()"));
}

#[test]
fn test_multiple_similarity_same_literal_vector_is_accepted() {
    // Two similarity() predicates referencing the SAME literal vector
    // must be accepted (identity of the VectorExpr is what matters, not
    // param vs. literal).
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, [1.0, 0.0, 0.0, 0.0]) > 0.5 AND similarity(vector, [1.0, 0.0, 0.0, 0.0]) < 0.999 LIMIT 10",
        None,
    )
    .expect("test: same-literal similarity range");
    assert_eq!(r.row_count(), 1);
    assert_eq!(r.row(0).expect("test: row").id(), 2);
}

#[test]
fn test_multiple_similarity_different_literals_returns_error() {
    // Two distinct literal vectors → same rejection.
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "SELECT * FROM vecs WHERE similarity(vector, [1.0, 0.0, 0.0, 0.0]) > 0.5 AND similarity(vector, [0.0, 1.0, 0.0, 0.0]) > 0.3 LIMIT 10",
        None,
    );
    assert!(err.is_err());
    assert!(err
        .expect_err("test: err")
        .contains("Multiple similarity()"));
}
