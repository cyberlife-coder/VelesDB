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

#[test]
fn test_near_and_id_in_returns_those_points_however_far_they_are() {
    // 200 points: 1..=3 point away from the query, 4..=200 toward it. A
    // post-filter over the nearest candidates finds none of 1..=3; `id IN`
    // names its candidates exactly.
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION many (dimension = 2, metric = 'cosine')",
    )
    .expect("test: create many");
    let vc = db.get_vector_collection("many").expect("test: get many");
    let points: Vec<Point> = (1..=200_u64)
        .map(|id| {
            let vector = if id <= 3 {
                vec![1.0, 0.0]
            } else {
                vec![0.0, 1.0]
            };
            Point::new(id, vector, Some(serde_json::json!({"id": 1000 + id})))
        })
        .collect();
    vc.upsert(points).expect("test: seed many");

    let query =
        Parser::parse("SELECT * FROM many WHERE vector NEAR $v AND id IN (1, 2, 3) LIMIT 3")
            .expect("test: parse");
    let params = HashMap::from([("v".to_string(), serde_json::json!([0.0, 1.0]))]);
    let mut ids: Vec<u64> = db
        .execute_query(&query, &params)
        .expect("test: NEAR AND id IN")
        .iter()
        .map(|r| r.point.id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn test_where_id_matches_a_point_without_payload_on_every_path() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);
    db.get_vector_collection("tickets")
        .expect("test: get tickets")
        .upsert(vec![Point::new(34, vec![0.5, 0.5], None)])
        .expect("test: payload-less point");

    assert_eq!(ids("SELECT * FROM tickets WHERE id = 34", &db), vec![34]);
    let query = Parser::parse("SELECT * FROM tickets WHERE vector NEAR $v AND id = 34 LIMIT 5")
        .expect("test: parse");
    let params = HashMap::from([("v".to_string(), serde_json::json!([0.5, 0.5]))]);
    let near: Vec<u64> = db
        .execute_query(&query, &params)
        .expect("test: NEAR AND id")
        .iter()
        .map(|r| r.point.id)
        .collect();
    assert_eq!(near, vec![34]);
}

#[test]
fn test_arithmetic_order_by_reads_id_as_the_point_id() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    assert_eq!(
        ids("SELECT * FROM tickets ORDER BY 0 - id ASC", &db),
        vec![33, 32, 31],
        "the payload ids (3, 1, 2) would give 31, 33, 32"
    );
}

#[test]
fn test_update_where_id_updates_that_point() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    execute_sql(&db, "UPDATE tickets SET kind = 'done' WHERE id IN (31, 3)").expect("test: update");

    let done = ids("SELECT * FROM tickets WHERE kind = 'done' ORDER BY id", &db);
    assert_eq!(done, vec![31], "3 is a payload id, not a point");
}

#[test]
fn test_join_on_id_reads_the_point_id_of_each_side() {
    // Each customer's payload carries the other's id: only the point id
    // pairs ticket 31's `owner` (10) with customer 10.
    let (_dir, db) = create_test_db();
    setup_tickets(&db);
    db.get_vector_collection("tickets")
        .expect("test: get tickets")
        .upsert(vec![Point::new(
            31,
            vec![1.0, 0.0],
            Some(serde_json::json!({"id": 3, "kind": "bug", "owner": 10})),
        )])
        .expect("test: owner");
    execute_sql(&db, "CREATE METADATA COLLECTION owners").expect("test: create owners");
    db.get_metadata_collection("owners")
        .expect("test: get owners")
        .upsert(vec![
            Point::metadata_only(10, serde_json::json!({"id": 20, "name": "Ten"})),
            Point::metadata_only(20, serde_json::json!({"id": 10, "name": "Twenty"})),
        ])
        .expect("test: seed owners");

    let rows = execute_sql(
        &db,
        "SELECT * FROM tickets JOIN owners ON tickets.owner = owners.id LIMIT 10",
    )
    .expect("test: join");

    let names: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.point.payload.as_ref()?.get("name")?.as_str())
        .collect();
    assert_eq!(names, vec!["Ten"]);
}

#[test]
fn test_update_reaches_a_point_without_payload_whatever_its_where() {
    // The scan universe is every point, not every point with a payload.
    let (_dir, db) = create_test_db();
    setup_tickets(&db);
    db.get_vector_collection("tickets")
        .expect("test: get tickets")
        .upsert(vec![
            Point::new(34, vec![0.5, 0.5], None),
            Point::new(35, vec![0.5, 0.5], None),
        ])
        .expect("test: payload-less points");

    execute_sql(&db, "UPDATE tickets SET u = 'y' WHERE NOT (id != 34)").expect("test: update");
    execute_sql(&db, "UPDATE tickets SET v = 'z' WHERE kind IS NULL").expect("test: update");

    assert_eq!(ids("SELECT * FROM tickets WHERE u = 'y'", &db), vec![34]);
    assert_eq!(
        ids("SELECT * FROM tickets WHERE v = 'z' ORDER BY id", &db),
        vec![34, 35]
    );
}

#[test]
fn test_window_functions_order_and_partition_by_the_point_id() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);
    // Point 35 shares payload id 3 with point 31.
    db.get_vector_collection("tickets")
        .expect("test: get tickets")
        .upsert(vec![Point::new(
            35,
            vec![0.5, 0.5],
            Some(serde_json::json!({"id": 3, "kind": "task"})),
        )])
        .expect("test: point 35");

    let rank = |sql: &str| -> Vec<(u64, u64)> {
        let mut rows: Vec<(u64, u64)> = execute_sql(&db, sql)
            .expect("test: window")
            .iter()
            .map(|r| {
                let rn = r.point.payload.as_ref().and_then(|p| p.get("rn")?.as_u64());
                (r.point.id, rn.expect("test: rn"))
            })
            .collect();
        rows.sort_unstable();
        rows
    };
    assert_eq!(
        rank("SELECT id, kind, ROW_NUMBER() OVER (ORDER BY id DESC) AS rn FROM tickets"),
        vec![(31, 4), (32, 3), (33, 2), (35, 1)]
    );
    assert_eq!(
        rank(
            "SELECT id, kind, ROW_NUMBER() OVER (PARTITION BY id ORDER BY kind) AS rn FROM tickets"
        ),
        vec![(31, 1), (32, 1), (33, 1), (35, 1)],
        "each point is its own partition; by payload id, 31 and 35 would share one"
    );
}

#[test]
fn test_an_aggregate_where_id_counts_only_existing_points() {
    let (_dir, db) = create_test_db();
    setup_tickets(&db);

    let one = aggregate(&db, "SELECT COUNT(*) AS n FROM tickets WHERE id = 31").expect("test");
    let none = aggregate(
        &db,
        "SELECT COUNT(*) AS n FROM tickets WHERE id IN (999, 1000)",
    )
    .expect("test");
    let grouped = aggregate(
        &db,
        "SELECT kind, COUNT(*) AS n FROM tickets WHERE id IN (31, 33, 999) GROUP BY kind",
    )
    .expect("test");

    assert_eq!(one.get("n"), Some(&serde_json::json!(1)));
    assert_eq!(none.get("n"), Some(&serde_json::json!(0)));
    assert_eq!(
        grouped,
        serde_json::json!([{"kind": "bug", "n": 1}, {"kind": "task", "n": 1}])
    );
}
