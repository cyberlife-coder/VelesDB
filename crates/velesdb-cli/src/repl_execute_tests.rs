//! Tests for the REPL query-execution path (`repl_execute`).

use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point};

use crate::repl_execute::execute_query;
use crate::session::SessionSettings;

/// Opens a fresh database with a single `docs` collection seeded with `n`
/// 2-D points, used by the projection/limit/param regression tests.
fn seed_docs(dir: &TempDir, n: u64) -> Database {
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .expect("create collection");
    let coll = db.get_vector_collection("docs").expect("vector collection");
    let points: Vec<Point> = (1..=n)
        .map(|i| {
            Point::new(
                i,
                vec![1.0, i as f32],
                Some(serde_json::json!({"category": "x"})),
            )
        })
        .collect();
    coll.upsert(points).expect("upsert");
    db
}

/// Regression (parity backlog #2): the REPL must route `GROUP BY` / aggregate
/// queries through the aggregate engine, not return raw rows. Previously
/// `execute_query` only forked match-vs-`Database::execute_query`, and the
/// standard SELECT projection returns empty rows for aggregate columns — so
/// `COUNT`/`GROUP BY`/`HAVING` were silently ignored in the REPL.
#[test]
fn test_execute_query_routes_group_by_having_aggregation() {
    let dir = TempDir::new().expect("temp dir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("orders", 2, DistanceMetric::Cosine)
        .expect("create collection");

    let coll = db
        .get_vector_collection("orders")
        .expect("vector collection");
    coll.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(serde_json::json!({"category": "a"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0],
            Some(serde_json::json!({"category": "a"})),
        ),
        Point::new(
            3,
            vec![1.0, 1.0],
            Some(serde_json::json!({"category": "b"})),
        ),
    ])
    .expect("upsert");

    let result = execute_query(
        &db,
        "SELECT category, COUNT(*) AS n FROM orders GROUP BY category HAVING COUNT(*) > 1",
        None,
        None,
    )
    .expect("aggregation query should succeed");

    // Only category 'a' (2 rows) clears HAVING COUNT(*) > 1; 'b' (1 row) drops.
    // Pre-fix the REPL returned all 3 raw rows with no aggregate column.
    assert_eq!(
        result.rows.len(),
        1,
        "expected exactly one aggregated group, got {:?}",
        result.rows
    );
    let row = &result.rows[0];
    assert_eq!(
        row.get("category"),
        Some(&serde_json::json!("a")),
        "group key must be category 'a'; row={row:?}"
    );
    assert_eq!(
        row.get("n").and_then(serde_json::Value::as_u64),
        Some(2),
        "COUNT(*) for category 'a' must be 2; row={row:?}"
    );
}

/// Regression (parity backlog #3): a plain `SELECT <col>` must project only the
/// requested column (matching the REST `/query` API), not return id + score +
/// the full payload. Pre-fix the REPL's `result_to_row` ignored the column list.
#[test]
fn test_execute_query_projects_selected_columns() {
    let dir = TempDir::new().expect("temp dir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .expect("create collection");
    let coll = db.get_vector_collection("docs").expect("vector collection");
    coll.upsert(vec![Point::new(
        1,
        vec![1.0, 0.0],
        Some(serde_json::json!({"title": "alpha", "category": "x"})),
    )])
    .expect("upsert");

    let result =
        execute_query(&db, "SELECT category FROM docs", None, None).expect("select should succeed");

    assert_eq!(result.rows.len(), 1, "one matching row");
    let row = &result.rows[0];
    assert_eq!(
        row.get("category"),
        Some(&serde_json::json!("x")),
        "the projected column must be present; row={row:?}"
    );
    assert!(
        !row.contains_key("title"),
        "a non-selected payload field must be dropped; row={row:?}"
    );
    assert!(
        !row.contains_key("id") && !row.contains_key("score"),
        "id/score must not be auto-appended for an explicit column list; row={row:?}"
    );
}

/// Regression (parity backlog #18a): a `$parameter` vector query the REPL cannot
/// supply must return `Err` (red error, non-zero exit), not `Ok(empty)` which
/// scripts silently treat as success. The helpful message text is preserved.
#[test]
fn test_param_vector_query_errors_instead_of_empty_ok() {
    let dir = TempDir::new().expect("temp dir");
    let db = seed_docs(&dir, 3);

    let result = execute_query(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $q LIMIT 5",
        None,
        None,
    );

    let err = result.expect_err("a $parameter vector query must error, not return empty Ok");
    assert!(
        err.to_string().contains("$parameter"),
        "the error must keep the helpful $parameter message; got: {err}"
    );
}

/// Regression (parity backlog #19): setting `ef_search=512` in the session must
/// inject `ef_search=512` into the AST WITH-options before execution.
#[test]
fn test_session_ef_search_injected_into_ast() {
    let mut session = SessionSettings::new();
    session.set("ef_search", "512").expect("set ef_search");

    let mut parsed =
        velesdb_core::velesql::Parser::parse("SELECT * FROM docs WHERE vector NEAR [1.0, 2.0]")
            .expect("parse");
    crate::repl_execute::apply_session_settings(&mut parsed, &session);

    let with = parsed
        .select
        .with_clause
        .as_ref()
        .expect("session ef_search must add a WITH clause");
    assert_eq!(
        with.get_ef_search(),
        Some(512),
        "session ef_search must reach the AST WITH-options"
    );
}

/// Regression (parity backlog #19): an inline `WITH(ef_search=N)` must win over
/// the session value (the session injects only when no inline override exists).
#[test]
fn test_inline_ef_search_wins_over_session() {
    let mut session = SessionSettings::new();
    session.set("ef_search", "512").expect("set ef_search");

    let mut parsed = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM docs WHERE vector NEAR [1.0, 2.0] WITH(ef_search=64)",
    )
    .expect("parse");
    crate::repl_execute::apply_session_settings(&mut parsed, &session);

    let with = parsed.select.with_clause.as_ref().expect("inline WITH");
    assert_eq!(
        with.get_ef_search(),
        Some(64),
        "inline WITH(ef_search) must win over the session value"
    );
}

/// Regression (parity backlog #19): `max_results` caps the effective LIMIT. A
/// query with no LIMIT and a session `max_results=2` must return at most 2 rows.
#[test]
fn test_session_max_results_caps_limit() {
    let dir = TempDir::new().expect("temp dir");
    let db = seed_docs(&dir, 5);
    let mut session = SessionSettings::new();
    session.set("max_results", "2").expect("set max_results");

    let result = execute_query(&db, "SELECT category FROM docs", None, Some(&session))
        .expect("select should succeed");

    assert!(
        result.rows.len() <= 2,
        "max_results=2 must cap the result count; got {} rows",
        result.rows.len()
    );
}

/// Regression (parity backlog #19): `\set` on an unwired key (`timeout_ms`,
/// `rerank`) must warn that the setting is display-only, instead of silently
/// claiming success.
#[test]
fn test_unwired_set_keys_are_flagged() {
    assert!(
        crate::session::is_unwired_setting("timeout_ms"),
        "timeout_ms has no channel into Database::execute_query and must warn"
    );
    assert!(
        crate::session::is_unwired_setting("rerank"),
        "rerank has no channel into Database::execute_query and must warn"
    );
    assert!(
        !crate::session::is_unwired_setting("ef_search"),
        "ef_search IS wired and must not warn"
    );
    assert!(
        !crate::session::is_unwired_setting("max_results"),
        "max_results IS wired and must not warn"
    );
}
