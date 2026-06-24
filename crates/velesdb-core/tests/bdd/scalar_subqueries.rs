//! BDD-style end-to-end tests for VelesQL **scalar subqueries** (EPIC-039).
//!
//! A scalar subquery `(SELECT AVG(amount) FROM t)` in a WHERE/HAVING predicate
//! (or an INSERT/UPDATE value) is executed, reduced to a single row / single
//! column, and substituted as a literal before the outer filter runs.
//!
//! These tests exercise the full pipeline: SQL string -> `Parser::parse()` ->
//! `Database::execute_query()` -> verify results. Data is seeded deterministically
//! so the resolved scalar is known up front.

use std::collections::HashMap;

use velesdb_core::{velesql::Parser, Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, payload_f64, result_ids,
};

/// Seed a 5-row `orders` collection with amounts 10/20/30/40/50.
///
/// `AVG(amount)` = 30, `MAX(amount)` = 50, `MIN(amount)` = 10, `COUNT(*)` = 5.
fn setup_orders(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION orders (dimension = 2, metric = 'cosine')",
    )
    .expect("test: create orders");

    let vc = db
        .get_vector_collection("orders")
        .expect("test: get orders");
    let amounts = [10.0_f64, 20.0, 30.0, 40.0, 50.0];
    let points: Vec<Point> = amounts
        .iter()
        .enumerate()
        .map(|(i, amount)| {
            let id = u64::try_from(i).expect("test: id fits u64") + 1;
            Point::new(
                id,
                vec![1.0, 0.0],
                Some(serde_json::json!({ "amount": amount })),
            )
        })
        .collect();
    vc.upsert(points).expect("test: seed orders");
}

/// `WHERE amount > (SELECT AVG(amount) FROM orders)` returns the rows above the
/// average (40, 50) — today this is rejected at validation with V010.
#[test]
fn where_scalar_subquery_avg_filters_above_average() {
    let (_dir, db) = create_test_db();
    setup_orders(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM orders WHERE amount > (SELECT AVG(amount) FROM orders)",
    )
    .expect("scalar subquery WHERE should execute");

    let ids = result_ids(&results);
    assert_eq!(ids, [4, 5].into_iter().collect(), "ids with amount > 30");
    for r in &results {
        assert!(
            payload_f64(r, "amount").expect("amount present") > 30.0,
            "every returned row is above the average"
        );
    }
}

/// A plain (non-aggregate) single-row, single-column subquery resolves too:
/// `WHERE amount >= (SELECT amount FROM orders WHERE amount = 40)`.
#[test]
fn where_scalar_subquery_plain_single_row() {
    let (_dir, db) = create_test_db();
    setup_orders(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM orders WHERE amount >= (SELECT amount FROM orders WHERE amount = 40)",
    )
    .expect("plain scalar subquery WHERE should execute");

    assert_eq!(result_ids(&results), [4, 5].into_iter().collect());
}

/// A subquery returning more than one row errors with a clear cardinality
/// message (Error::Query), not a silent wrong result.
#[test]
fn where_scalar_subquery_multi_row_errors() {
    let (_dir, db) = create_test_db();
    setup_orders(&db);

    let query = Parser::parse("SELECT * FROM orders WHERE amount > (SELECT amount FROM orders)")
        .expect("parses");
    let err = db
        .execute_query(&query, &HashMap::new())
        .expect_err("multi-row subquery must error");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("one row") || msg.contains("more than one") || msg.contains("cardinality"),
        "error message names the cardinality violation: {msg}"
    );
}

/// A zero-row subquery resolves to NULL; `amount > NULL` is never true, so the
/// outer query returns no rows (documented behavior).
#[test]
fn where_scalar_subquery_zero_rows_yields_null() {
    let (_dir, db) = create_test_db();
    setup_orders(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM orders WHERE amount > (SELECT amount FROM orders WHERE amount = 9999)",
    )
    .expect("zero-row subquery resolves to NULL");

    assert!(
        results.is_empty(),
        "comparison against NULL yields no rows, got {}",
        results.len()
    );
}

/// A `SELECT *` subquery violates the one-column contract and errors clearly.
#[test]
fn where_scalar_subquery_star_projection_errors() {
    let (_dir, db) = create_test_db();
    setup_orders(&db);

    let query = Parser::parse(
        "SELECT * FROM orders WHERE amount > (SELECT * FROM orders WHERE amount = 40)",
    )
    .expect("parses");
    let err = db
        .execute_query(&query, &HashMap::new())
        .expect_err("SELECT * subquery must error on the one-column contract");
    assert!(
        err.to_string().to_lowercase().contains("one column"),
        "error names the one-column violation: {err}"
    );
}

/// A HAVING scalar subquery on a top-level aggregate query resolves and filters
/// groups: `HAVING COUNT(*) > (SELECT COUNT(*) ... )`.
///
/// Seed: amounts 10/20/30 in category 'a', 40/50 in 'b'. The subquery
/// `(SELECT COUNT(*) FROM orders WHERE amount > 35)` = 2. `HAVING COUNT(*) >= 2`
/// keeps only 'a' (3 rows); 'b' (2 rows) is dropped by `> 2`.
#[test]
fn having_scalar_subquery_filters_groups() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION cats (dimension = 2, metric = 'cosine')",
    )
    .expect("test: create cats");
    let vc = db.get_vector_collection("cats").expect("test: get cats");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(serde_json::json!({"cat":"a","amount":10.0})),
        ),
        Point::new(
            2,
            vec![1.0, 0.0],
            Some(serde_json::json!({"cat":"a","amount":20.0})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0],
            Some(serde_json::json!({"cat":"a","amount":30.0})),
        ),
        Point::new(
            4,
            vec![1.0, 0.0],
            Some(serde_json::json!({"cat":"b","amount":40.0})),
        ),
        Point::new(
            5,
            vec![1.0, 0.0],
            Some(serde_json::json!({"cat":"b","amount":50.0})),
        ),
    ])
    .expect("test: seed cats");

    let query = Parser::parse(
        "SELECT cat, COUNT(*) FROM cats GROUP BY cat \
         HAVING COUNT(*) > (SELECT COUNT(*) FROM cats WHERE amount > 35)",
    )
    .expect("parses");
    let json = db
        .execute_aggregate(&query, &HashMap::new())
        .expect("HAVING subquery aggregate should execute");

    let groups = json.as_array().expect("grouped result is an array");
    assert_eq!(groups.len(), 1, "only category 'a' has > 2 rows: {json}");
    assert_eq!(groups[0].get("cat").and_then(|v| v.as_str()), Some("a"));
}

/// An INSERT VALUES scalar subquery resolves before the row is written.
#[test]
fn insert_value_scalar_subquery_resolves() {
    let (_dir, db) = create_test_db();
    setup_orders(&db);

    // amount := MAX(amount) = 50 for the new id=6 row.
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0_f32, 0.0_f32]));
    execute_sql_with_params(
        &db,
        "INSERT INTO orders (id, vector, amount) \
         VALUES (6, $v, (SELECT MAX(amount) FROM orders))",
        &params,
    )
    .expect("INSERT with scalar subquery value should execute");

    let inserted = execute_sql(&db, "SELECT * FROM orders WHERE amount = 50 LIMIT 100")
        .expect("read back inserted row");
    let new_row = inserted
        .iter()
        .find(|r| r.point.id == 6)
        .expect("the new id=6 row exists");
    assert_eq!(
        payload_f64(new_row, "amount"),
        Some(50.0),
        "amount was filled from MAX(amount)"
    );
}

/// Collect the ids of `orders` rows whose `flagged` payload field is `true`.
fn flagged_ids(db: &Database) -> std::collections::HashSet<u64> {
    let rows = execute_sql(db, "SELECT * FROM orders LIMIT 100").expect("read back orders");
    rows.iter()
        .filter(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("flagged"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
        })
        .map(|r| r.point.id)
        .collect()
}

/// CRITICAL (backlog-#21): `UPDATE ... SET flagged = true WHERE amount >
/// (SELECT AVG(amount) FROM orders)` must flag exactly the rows above the
/// average (40, 50) and leave the others untouched.
///
/// On the pre-completion code the WHERE subquery is never substituted, so the
/// predicate evaluates `amount > NULL` = false and the UPDATE silently matches
/// nothing — this test FAILS there (empty flagged set) and PASSES once the
/// subquery is resolved before the UPDATE runs.
#[test]
fn update_where_scalar_subquery_flags_above_average() {
    let (_dir, db) = create_test_db();
    setup_orders(&db);

    execute_sql(
        &db,
        "UPDATE orders SET flagged = true WHERE amount > (SELECT AVG(amount) FROM orders)",
    )
    .expect("UPDATE WHERE scalar subquery should execute");

    assert_eq!(
        flagged_ids(&db),
        [4, 5].into_iter().collect(),
        "exactly the rows with amount > AVG(30) were flagged"
    );
}

/// DELETE consistency: `DELETE FROM orders WHERE id = (SELECT oid ...)` resolves
/// the subquery to a literal integer id and deletes exactly that row, rather
/// than silently no-op'ing or rejecting it.
///
/// (`DELETE WHERE id = N` requires an integer literal; an *aggregate* subquery
/// like `MAX(id)` yields a float and is out of scope here, so the discriminating
/// subquery is a single-row projection of an integer payload field.)
#[test]
fn delete_where_scalar_subquery_resolves_id() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION orders (dimension = 2, metric = 'cosine')",
    )
    .expect("create orders");
    let vc = db.get_vector_collection("orders").expect("get orders");
    // Each row carries an integer `oid` payload equal to its point id.
    let points: Vec<Point> = (1_u64..=4)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.0],
                Some(serde_json::json!({ "oid": id, "tag": id })),
            )
        })
        .collect();
    vc.upsert(points).expect("seed orders");

    // (SELECT oid FROM orders WHERE tag = 3) = 3 -> deletes row id=3.
    execute_sql(
        &db,
        "DELETE FROM orders WHERE id = (SELECT oid FROM orders WHERE tag = 3)",
    )
    .expect("DELETE WHERE scalar subquery should execute");

    let remaining = execute_sql(&db, "SELECT * FROM orders LIMIT 100").expect("read back orders");
    assert_eq!(
        result_ids(&remaining),
        [1, 2, 4].into_iter().collect(),
        "only the row whose oid matched the subquery (id=3) was deleted"
    );
}

/// MEDIUM: a subquery whose inner WHERE filters on a dotted **payload path**
/// (`meta.cat`) is a plain payload filter, not a correlation — it must be
/// ACCEPTED and executed, not wrongly rejected as correlated (V010).
#[test]
fn payload_path_subquery_is_not_correlated() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION sub (dimension = 2, metric = 'cosine')",
    )
    .expect("create sub");
    let vc = db.get_vector_collection("sub").expect("get sub");
    // Two rows with meta.cat = 5 (prices 10, 30 -> AVG 20) and one with cat = 1.
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(serde_json::json!({"price": 10.0, "meta": {"cat": 5}})),
        ),
        Point::new(
            2,
            vec![1.0, 0.0],
            Some(serde_json::json!({"price": 30.0, "meta": {"cat": 5}})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0],
            Some(serde_json::json!({"price": 99.0, "meta": {"cat": 1}})),
        ),
    ])
    .expect("seed sub");

    // AVG(price WHERE meta.cat = 5) = 20; outer keeps prices > 20 (id 2 and 3).
    let results = execute_sql(
        &db,
        "SELECT * FROM sub WHERE price > (SELECT AVG(price) FROM sub WHERE meta.cat = 5)",
    )
    .expect("payload-path subquery must be accepted and executed, not rejected as correlated");

    assert_eq!(result_ids(&results), [2, 3].into_iter().collect());
}

/// A genuinely correlated subquery — its inner WHERE references the *outer*
/// table (`sub.price`) — is still rejected at validation with V010.
#[test]
fn genuinely_correlated_subquery_is_still_rejected() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION sub (dimension = 2, metric = 'cosine')",
    )
    .expect("create sub");

    let query = Parser::parse(
        "SELECT * FROM sub WHERE price > (SELECT AVG(price) FROM inner_t WHERE sub.price = 5)",
    )
    .expect("parses");
    let err = db
        .execute_query(&query, &HashMap::new())
        .expect_err("correlated subquery (referencing outer table) must be rejected");
    assert!(
        err.to_string().to_lowercase().contains("correlated"),
        "rejection names the correlation: {err}"
    );
}
