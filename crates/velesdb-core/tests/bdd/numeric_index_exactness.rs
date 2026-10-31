//! BDD-style end-to-end tests: an index never drops a true row nor adds a
//! false one. Payload indexes key numbers by `f64`, which cannot tell 2^53
//! from 2^53 + 1; the engine compares integers exactly, so an index path
//! must fall back to the exact one there. And the point id is the engine's,
//! never an index's.

use std::collections::HashMap;

use velesdb_core::{velesql::Parser, Database, Point};

use super::helpers::{create_test_db, execute_sql};

/// 2^53, the first integer whose successor shares its `f64`.
const TWO_53: u64 = 9_007_199_254_740_992;

fn ids(db: &Database, sql: &str) -> Vec<u64> {
    execute_sql(db, sql)
        .expect("test: select")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

fn aggregate(db: &Database, sql: &str) -> serde_json::Value {
    let query = Parser::parse(sql).expect("test: parse");
    db.execute_aggregate(&query, &HashMap::new())
        .expect("test: aggregate")
}

/// `big`: point 1 has `n = 2^53 + 1`, point 2 `n = 2^53`, point 3 `n = 7`.
fn setup_big(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION big (dimension = 2, metric = 'cosine')",
    )
    .expect("test: create big");
    let vc = db.get_vector_collection("big").expect("test: get big");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(serde_json::json!({"n": TWO_53 + 1, "g": "a"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0],
            Some(serde_json::json!({"n": TWO_53, "g": "b"})),
        ),
        Point::new(
            3,
            vec![1.0, 1.0],
            Some(serde_json::json!({"n": 7, "g": "c"})),
        ),
    ])
    .expect("test: seed big");
}

#[test]
fn test_order_by_an_indexed_field_is_exact_beyond_2_pow_53() {
    let (_dir, db) = create_test_db();
    setup_big(&db);
    let unindexed = ids(&db, "SELECT * FROM big ORDER BY n ASC LIMIT 2");
    execute_sql(&db, "CREATE INDEX ON big (n)").expect("test: index n");

    let indexed = ids(&db, "SELECT * FROM big ORDER BY n ASC LIMIT 2");

    assert_eq!(unindexed, vec![3, 2]);
    assert_eq!(indexed, vec![3, 2], "the index tied 2^53 with 2^53 + 1");
    assert_eq!(
        ids(&db, "SELECT * FROM big ORDER BY n DESC LIMIT 1"),
        vec![1]
    );
}

#[test]
fn test_an_indexed_join_on_id_never_pairs_2_pow_53_with_its_successor() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION lefts (dimension = 2, metric = 'cosine')",
    )
    .expect("test: create lefts");
    db.get_vector_collection("lefts")
        .expect("test: get lefts")
        .upsert(vec![Point::new(TWO_53 + 1, vec![1.0, 0.0], None)])
        .expect("test: seed lefts");
    execute_sql(&db, "CREATE METADATA COLLECTION rights").expect("test: create rights");
    db.get_metadata_collection("rights")
        .expect("test: get rights")
        .upsert(vec![Point::metadata_only(
            1,
            serde_json::json!({"lid": TWO_53, "name": "two53"}),
        )])
        .expect("test: seed rights");
    execute_sql(&db, "CREATE INDEX ON rights (lid)").expect("test: index lid");

    let rows = execute_sql(
        &db,
        "SELECT * FROM lefts JOIN rights ON rights.lid = lefts.id LIMIT 10",
    )
    .expect("test: join");

    assert!(rows.is_empty(), "2^53 + 1 joined 2^53: {rows:?}");
}

#[test]
fn test_a_filter_orders_an_integer_and_a_float_exactly() {
    let (_dir, db) = create_test_db();
    setup_big(&db);

    assert_eq!(
        ids(
            &db,
            "SELECT * FROM big WHERE n > 9007199254740992.0 ORDER BY id"
        ),
        vec![1],
        "2^53 + 1 is greater than the float 2^53"
    );
}

#[test]
fn test_min_and_max_keep_an_integer_exact() {
    let (_dir, db) = create_test_db();
    setup_big(&db);

    let extremes = aggregate(&db, "SELECT MIN(n) AS low, MAX(n) AS top FROM big");
    assert_eq!(extremes.get("top"), Some(&serde_json::json!(TWO_53 + 1)));
    assert_eq!(extremes.get("low"), Some(&serde_json::json!(7)));

    let having = aggregate(
        &db,
        "SELECT g, MAX(n) AS top FROM big GROUP BY g HAVING MAX(n) = 9007199254740992",
    );
    assert_eq!(
        having,
        serde_json::json!([{"g": "b", "top": TWO_53}]),
        "only the group whose maximum is 2^53"
    );
}

#[test]
fn test_the_point_id_field_name_cannot_be_indexed() {
    // An index under the internal point-id name would answer `id` ranges
    // from payloads.
    let (_dir, db) = create_test_db();
    setup_big(&db);

    let err = db
        .get_vector_collection("big")
        .expect("test: get big")
        .create_index("@velesql.point_id")
        .expect_err("test: reserved name");
    assert!(err.to_string().contains("@velesql.point_id"), "got: {err}");
    assert_eq!(
        ids(&db, "SELECT * FROM big WHERE id > 1 ORDER BY id"),
        vec![2, 3]
    );
}
