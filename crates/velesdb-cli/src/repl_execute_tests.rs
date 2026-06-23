//! Tests for the REPL query-execution path (`repl_execute`).

use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point};

use crate::repl_execute::execute_query;

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
        execute_query(&db, "SELECT category FROM docs", None).expect("select should succeed");

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
