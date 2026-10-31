//! BDD-style end-to-end tests: `id` is the point id in every clause of a
//! `VelesQL` SELECT — WHERE, projection, ORDER BY, GROUP BY and aggregates —
//! even when the payload carries a field of that name.

use std::collections::HashMap;

use velesdb_core::{velesql::Parser, Database, Point};

use super::helpers::{create_test_db, execute_sql};

/// Seeds `tickets`: points 31, 32 and 33, each carrying a payload `id` that
/// is not its point id (3, 1, 2), and a `kind`.
fn setup_tickets(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION tickets (dimension = 2, metric = 'cosine')",
    )
    .expect("test: create tickets");
    let vc = db
        .get_vector_collection("tickets")
        .expect("test: get tickets");
    vc.upsert(vec![
        Point::new(
            31,
            vec![1.0, 0.0],
            Some(serde_json::json!({"id": 3, "kind": "bug"})),
        ),
        Point::new(
            32,
            vec![0.0, 1.0],
            Some(serde_json::json!({"id": 1, "kind": "bug"})),
        ),
        Point::new(
            33,
            vec![1.0, 1.0],
            Some(serde_json::json!({"id": 2, "kind": "task"})),
        ),
    ])
    .expect("test: seed tickets");
}

fn aggregate(db: &Database, sql: &str) -> velesdb_core::Result<serde_json::Value> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.execute_aggregate(&query, &HashMap::new())
}

fn ids(sql: &str, db: &Database) -> Vec<u64> {
    execute_sql(db, sql)
        .expect("test: select")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

#[test]
fn test_where_id_reads_the_point_id_not_a_payload_field_named_id() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    assert_eq!(
        ids("SELECT * FROM tickets WHERE id > 31 ORDER BY id DESC", &db),
        vec![33, 32],
        "WHERE and ORDER BY read the point id"
    );
    assert_eq!(
        ids(
            "SELECT * FROM tickets WHERE id IN (31, 33) ORDER BY id",
            &db
        ),
        vec![31, 33]
    );
}

#[test]
fn test_order_by_id_sorts_on_the_point_id() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    assert_eq!(
        ids("SELECT * FROM tickets ORDER BY id ASC", &db),
        vec![31, 32, 33],
        "the payload ids (3, 1, 2) would give 32, 33, 31"
    );
}

#[test]
fn test_count_of_id_filtered_by_id_reads_the_point_id_in_both_clauses() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    let result = aggregate(
        &db,
        "SELECT COUNT(id) AS n, MAX(id) AS top FROM tickets WHERE id < 33",
    )
    .expect("test: aggregate");

    // The payload ids (3, 1, 2) all pass `< 33`: reading them counts 3.
    assert_eq!(result.get("n"), Some(&serde_json::json!(2)));
    assert_eq!(result.get("top"), Some(&serde_json::json!(32.0)));
}

#[test]
fn test_a_grouped_aggregate_filters_by_the_point_id() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    let result = aggregate(
        &db,
        "SELECT kind, COUNT(*) AS n FROM tickets WHERE id >= 32 GROUP BY kind",
    )
    .expect("test: aggregate");

    assert_eq!(
        result,
        serde_json::json!([{"kind": "bug", "n": 1}, {"kind": "task", "n": 1}])
    );
}

#[test]
fn test_execute_aggregate_refuses_a_vector_group_by() {
    // A vector GROUP BY groups search results: run as an aggregate it would
    // group the whole collection with no group budget.
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    let query =
        Parser::parse("SELECT kind, COUNT(*) AS n FROM tickets WHERE vector NEAR $v GROUP BY kind")
            .expect("test: parse");
    let params = HashMap::from([("v".to_string(), serde_json::json!([1.0, 0.0]))]);
    let err = db
        .execute_aggregate(&query, &params)
        .expect_err("test: a vector GROUP BY is a search");
    assert!(err.to_string().contains("execute_query"), "got: {err}");
}
