//! Behavior-parity tests for the WASM VelesQL executor (backlog #3b/#8/#9).
//!
//! These pin WASM SELECT semantics to velesdb-core's single source of truth:
//!
//! - **#3b** — column projection, `AS` aliases, and window functions on plain /
//!   vector SELECT (every SELECT previously returned the full payload).
//! - **#8** — `ORDER BY` arithmetic / `similarity(field, $v)` (previously a
//!   silent scan-order no-op) and the default `SELECT` LIMIT (previously
//!   unbounded).
//!
//! The #9 single-branch FUSION regression lives in `velesql_exec_hybrid_tests`
//! alongside the other FUSION coverage.

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

/// Parses every row in a `QueryResult` into a JSON object vector.
fn rows_as_objects(result: &crate::velesql_result::QueryResult) -> Vec<serde_json::Value> {
    let json = result.rows_json();
    serde_json::from_str(&json).expect("test: rows_json must be a JSON array")
}

fn db_with_docs() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs").expect("test: create");
    execute(
        &mut db,
        "INSERT INTO docs (id, category, title) VALUES \
         (1, 'tech', 'Alpha'), (2, 'food', 'Bravo'), (3, 'tech', 'Charlie')",
        None,
    )
    .expect("test: seed docs");
    db
}

fn db_with_vectors() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    for (id, v, price) in [
        (1u64, "[1.0, 0.0, 0.0, 0.0]", 10.0),
        (2, "[0.9, 0.1, 0.0, 0.0]", 5.0),
        (3, "[0.0, 1.0, 0.0, 0.0]", 30.0),
        (4, "[0.0, 0.0, 1.0, 0.0]", 1.0),
    ] {
        execute(
            &mut db,
            &format!(
                "INSERT INTO vecs (id, vector, price, cat) VALUES ({id}, $v, {price}, 'tech')"
            ),
            Some(&format!("{{\"v\": {v}}}")),
        )
        .expect("test: seed");
    }
    db
}

// =========================================================================
// #3b — projection / aliases / window functions
// =========================================================================

#[test]
fn test_select_single_column_projects_only_that_column() {
    let mut db = db_with_docs();
    let r = execute(&mut db, "SELECT category FROM docs", None).expect("test: select");
    let rows = rows_as_objects(&r);
    assert_eq!(rows.len(), 3);
    for row in &rows {
        let obj = row.as_object().expect("test: object");
        assert_eq!(
            obj.keys().collect::<Vec<_>>(),
            vec!["category"],
            "row must contain ONLY the projected column, got {obj:?}"
        );
    }
}

#[test]
fn test_select_alias_renames_column() {
    let mut db = db_with_docs();
    let r = execute(&mut db, "SELECT title AS name FROM docs", None).expect("test: select");
    let rows = rows_as_objects(&r);
    assert_eq!(rows.len(), 3);
    for row in &rows {
        let obj = row.as_object().expect("test: object");
        assert!(obj.contains_key("name"), "alias `name` must be present");
        assert!(
            !obj.contains_key("title"),
            "original `title` must be dropped"
        );
    }
}

#[test]
fn test_select_window_function_includes_rank_column() {
    let mut db = db_with_vectors();
    let r = execute(
        &mut db,
        "SELECT cat, ROW_NUMBER() OVER (ORDER BY price) AS rn FROM vecs",
        None,
    )
    .expect("test: window");
    let rows = rows_as_objects(&r);
    assert_eq!(rows.len(), 4);
    for row in &rows {
        let obj = row.as_object().expect("test: object");
        assert!(obj.contains_key("rn"), "window alias `rn` must be present");
    }
    // The smallest price (id=4, price=1.0) must rank 1.
    let ranks: Vec<u64> = rows
        .iter()
        .map(|r| r["rn"].as_u64().expect("test: rn u64"))
        .collect();
    assert!(ranks.contains(&1) && ranks.contains(&4));
}

// =========================================================================
// #8 — ORDER BY arithmetic / similarity + default LIMIT
// =========================================================================

#[test]
fn test_select_without_limit_caps_at_default() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("big").expect("test: create");
    for i in 1u64..=25 {
        execute(
            &mut db,
            &format!("INSERT INTO big (id, n) VALUES ({i}, {i})"),
            None,
        )
        .expect("test: seed");
    }
    let r = execute(&mut db, "SELECT * FROM big", None).expect("test: select");
    assert_eq!(
        r.row_count(),
        10,
        "a LIMIT-less SELECT must cap at DEFAULT_SELECT_LIMIT (10), not return all 25"
    );
}

#[test]
fn test_order_by_arithmetic_sorts_by_formula() {
    let mut db = db_with_vectors();
    // price - 2*score ASC. With $q=[1,0,0,0]: score≈[1,2:0.994,3:0,4:0].
    // key = price - 2*score:
    //   id1: 10 - 2.0   = 8.0
    //   id2: 5  - 1.989 = 3.011
    //   id3: 30 - 0     = 30
    //   id4: 1  - 0     = 1
    // ASC order: id4 (1) < id2 (3.011) < id1 (8) < id3 (30)
    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q ORDER BY (price - 2 * score) ASC",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: arithmetic order");
    let ids: Vec<u64> = (0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect();
    assert_eq!(ids, vec![4, 2, 1, 3], "rows must be in the formula order");
}

#[test]
fn test_order_by_similarity_named_field_rejected_loudly() {
    // WASM has no named/secondary vectors. `ORDER BY similarity(image_vec, $q)`
    // cannot be evaluated and must reject loudly (parity with the MATCH path),
    // never silently return scan order.
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q ORDER BY similarity(image_vec, $q) DESC",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    );
    assert!(err.is_err(), "named-vector similarity ORDER BY must error");
}
