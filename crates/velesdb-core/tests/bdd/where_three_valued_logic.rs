//! BDD scenarios: WHERE evaluates in SQL's three-valued logic.
//!
//! A predicate on a field that is `null` or absent is UNKNOWN, not false, and
//! WHERE admits only rows whose condition is known TRUE. The difference shows
//! under negation: `NOT (x = 'a')` and `x != 'a'` exclude a row without `x`,
//! because `NOT UNKNOWN` is UNKNOWN. Every path — the payload scan, an
//! indexed field, a filtered vector search, a graph-predicate WHERE and a
//! MATCH — must return the same rows.

use std::collections::{HashMap, HashSet};

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, result_ids};

/// | id | x      | n    |
/// |----|--------|------|
/// | 1  | 'a'    | 1    |
/// | 2  | 'b'    | 2    |
/// | 3  | null   | null |
/// | 4  | (none) |(none)|
/// | 5  | 5      | (none)|
fn payloads() -> Vec<(u64, serde_json::Value)> {
    vec![
        (1, json!({"x": "a", "n": 1})),
        (2, json!({"x": "b", "n": 2})),
        (3, json!({"x": null, "n": null})),
        (4, json!({})),
        (5, json!({"x": 5})),
    ]
}

fn setup(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION t (dimension = 2, metric = 'cosine');",
    )
    .expect("test: CREATE t");
    let vc = db.get_vector_collection("t").expect("test: t exists");
    let points: Vec<Point> = payloads()
        .into_iter()
        .map(|(id, payload)| Point::new(id, vec![1.0, 0.0], Some(payload)))
        .collect();
    vc.upsert(points).expect("test: upsert t");
}

fn ids(set: &[u64]) -> HashSet<u64> {
    set.iter().copied().collect()
}

/// (WHERE condition template, ids known TRUE for it). `{x}` and `{n}` stand
/// for the two columns, so MATCH can qualify them with its node alias.
fn cases() -> Vec<(&'static str, HashSet<u64>)> {
    vec![
        ("{x} != 'a'", ids(&[2, 5])),
        ("NOT ({x} = 'a')", ids(&[2, 5])),
        ("{x} NOT IN ('a')", ids(&[2, 5])),
        ("NOT ({x} IN ('a'))", ids(&[2, 5])),
        ("{x} IN ('a', NULL)", ids(&[1])),
        ("{x} NOT IN ('a', NULL)", ids(&[])),
        ("{x} = NULL", ids(&[])),
        ("{x} != NULL", ids(&[])),
        ("{x} = 'a' OR {x} != 'a'", ids(&[1, 2, 5])),
        ("NOT ({x} = 'a') OR {x} IS NULL", ids(&[2, 3, 4, 5])),
        ("NOT ({x} = 'a' AND {n} = 1)", ids(&[2, 5])),
        ("NOT ({x} = 'b' OR {n} = 1)", ids(&[])),
        ("NOT ({n} > 1)", ids(&[1])),
        ("NOT ({n} BETWEEN 2 AND 3)", ids(&[1])),
        ("NOT ({x} LIKE 'a%')", ids(&[2, 5])),
        ("NOT (NOT ({x} = 'a'))", ids(&[1])),
        ("{x} IS NULL", ids(&[3, 4])),
        ("NOT ({x} IS NULL)", ids(&[1, 2, 5])),
    ]
}

/// Instantiates every case template with the columns `x` and `n` as
/// `prefix` qualifies them (`""` for SELECT, `"i."` for a MATCH alias).
fn conditions(prefix: &str) -> Vec<(String, HashSet<u64>)> {
    cases()
        .into_iter()
        .map(|(template, expected)| {
            let cond = template
                .replace("{x}", &format!("{prefix}x"))
                .replace("{n}", &format!("{prefix}n"));
            (cond, expected)
        })
        .collect()
}

/// GIVEN rows where `x` is a string, a number, `null` or absent
/// WHEN a SELECT filters them with negated or `NULL`-comparing predicates
/// THEN only rows whose condition is known TRUE come back.
#[test]
fn test_where_admits_only_known_true_rows() {
    let (_dir, db) = create_test_db();
    setup(&db);
    for (cond, expected) in conditions("") {
        let sql = format!("SELECT * FROM t WHERE {cond} LIMIT 100");
        let got = result_ids(&execute_sql(&db, &sql).expect("test: query"));
        assert_eq!(got, expected, "{sql}");
    }
}

/// GIVEN the same rows with a secondary index on `x`
/// WHEN the same predicates run
/// THEN the index path returns exactly the scan's rows.
#[test]
fn test_where_three_valued_logic_holds_with_an_index() {
    let (_dir, db) = create_test_db();
    setup(&db);
    execute_sql(&db, "CREATE INDEX ON t (x)").expect("test: CREATE INDEX");
    for (cond, expected) in conditions("") {
        let sql = format!("SELECT * FROM t WHERE {cond} LIMIT 100");
        let got = result_ids(&execute_sql(&db, &sql).expect("test: query"));
        assert_eq!(got, expected, "indexed: {sql}");
    }
}

/// GIVEN the same rows
/// WHEN the predicates filter a vector search
/// THEN the filtered search returns exactly the scan's rows.
#[test]
fn test_where_three_valued_logic_holds_in_a_filtered_vector_search() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let params: HashMap<String, serde_json::Value> =
        HashMap::from([("v".to_string(), json!([1.0, 0.0]))]);
    for (cond, expected) in conditions("") {
        let sql = format!("SELECT * FROM t WHERE vector NEAR $v AND ({cond}) LIMIT 100");
        let got = result_ids(&execute_sql_with_params(&db, &sql, &params).expect("test: query"));
        assert_eq!(got, expected, "vector search: {sql}");
    }
}

/// GIVEN the same rows
/// WHEN the predicates sit beside a `similarity()` threshold every row passes
/// THEN the three-valued WHERE evaluator keeps a metadata UNKNOWN unknown.
#[test]
fn test_where_three_valued_logic_holds_beside_similarity() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let params: HashMap<String, serde_json::Value> =
        HashMap::from([("v".to_string(), json!([1.0, 0.0]))]);
    for (cond, expected) in conditions("") {
        let sql =
            format!("SELECT * FROM t WHERE similarity(vector, $v) > 0.5 AND ({cond}) LIMIT 100");
        let got = result_ids(&execute_sql_with_params(&db, &sql, &params).expect("test: query"));
        assert_eq!(got, expected, "similarity: {sql}");
    }
}

/// GIVEN graph nodes carrying the same payloads
/// WHEN a MATCH WHERE applies the predicates to the node alias
/// THEN MATCH returns exactly the SELECT rows.
#[test]
fn test_match_where_three_valued_logic_matches_select() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION g (dimension = 2, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE GRAPH g");
    let gc = db.get_graph_collection("g").expect("test: g exists");
    for (id, mut payload) in payloads() {
        payload["_labels"] = json!(["Item"]);
        gc.upsert_node_payload(id, &payload).expect("test: node");
    }
    let params: HashMap<String, serde_json::Value> =
        HashMap::from([("_collection".to_string(), json!("g"))]);
    for (cond, expected) in conditions("i.") {
        let sql = format!("MATCH (i:Item) WHERE {cond} RETURN i LIMIT 100");
        let got = result_ids(&execute_sql_with_params(&db, &sql, &params).expect("test: query"));
        assert_eq!(got, expected, "MATCH: {sql}");
    }
}
