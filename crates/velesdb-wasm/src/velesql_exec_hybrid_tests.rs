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
