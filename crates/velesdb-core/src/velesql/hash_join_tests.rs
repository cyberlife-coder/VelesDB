use super::*;

use serde_json::json;

use crate::velesql::{scope_base, Parser};

/// The first JOIN clause of `sql`.
fn join_of(sql: &str) -> JoinClause {
    Parser::parse(sql)
        .expect("test: parse")
        .select
        .joins
        .into_iter()
        .next()
        .expect("test: one JOIN")
}

/// A base row of `o` as the engines build it before any join.
fn order(id: u64, payload: serde_json::Value) -> SearchResult {
    SearchResult::new(
        Point::metadata_only(id, scope_base(Some(payload), "o", id)),
        1.0,
    )
}

fn customer(id: u64, payload: serde_json::Value) -> Point {
    Point::metadata_only(id, payload)
}

fn run(sql: &str, rows: Vec<SearchResult>, side: &[Point]) -> Vec<SearchResult> {
    let join = join_of(sql);
    let keys = JoinKeys::of(&join).expect("test: join keys");
    hash_join(rows, side, &join, &keys, &["o"], usize::MAX)
}

/// `(row id, joined id)` of each output row.
fn pairs(rows: &[SearchResult]) -> Vec<(u64, Option<u64>)> {
    rows.iter()
        .map(|r| {
            let payload = r.point.payload.as_ref().expect("test: payload");
            (r.point.id, payload["c"]["id"].as_u64())
        })
        .collect()
}

fn orders() -> Vec<SearchResult> {
    vec![
        order(1, json!({"cust": "ana"})),
        order(2, json!({"cust": "bob"})),
        order(3, json!({"cust": null})),
    ]
}

fn customers() -> Vec<Point> {
    vec![
        customer(10, json!({"handle": "ana", "city": "Lyon"})),
        customer(11, json!({"handle": "ana", "city": "Nice"})),
        customer(12, json!({"handle": "zoe"})),
    ]
}

const ON_HANDLE: &str = "SELECT * FROM o JOIN c ON o.cust = c.handle";

/// GIVEN orders and customers keyed by an unindexed string column
/// WHEN they INNER JOIN on it
/// THEN each order pairs with every customer of that handle, and an order
///      without one (or with a NULL key) is dropped.
#[test]
fn inner_join_matches_any_column_one_row_per_match() {
    let rows = run(ON_HANDLE, orders(), &customers());
    assert_eq!(pairs(&rows), [(1, Some(10)), (1, Some(11))]);
    let first = rows[0].point.payload.as_ref().expect("test: payload");
    assert_eq!(first["c"]["city"], json!("Lyon"));
    assert_eq!(first["city"], json!("Lyon"), "the side is merged flat too");
}

/// A row without a match carries every column of the joined collection,
/// its `id` included, as `null`, flat and under the alias.
#[test]
fn left_join_keeps_each_row_without_a_match() {
    let sql = ON_HANDLE.replace("JOIN", "LEFT JOIN");
    let rows = run(&sql, orders(), &customers());
    assert_eq!(
        pairs(&rows),
        [(1, Some(10)), (1, Some(11)), (2, None), (3, None)]
    );
    let unmatched = rows[2].point.payload.as_ref().expect("test: payload");
    assert_eq!(unmatched["city"], serde_json::Value::Null);
    assert_eq!(unmatched["c"]["handle"], serde_json::Value::Null);
    assert_eq!(unmatched["c"]["id"], serde_json::Value::Null);
    assert_eq!(unmatched["cust"], json!("bob"), "the base fields stay");
}

/// RIGHT adds each joined point no row matched, as a row of its own whose
/// base side carries a `null` id.
#[test]
fn right_join_adds_each_unmatched_point() {
    let sql = ON_HANDLE.replace("JOIN", "RIGHT JOIN");
    let rows = run(&sql, orders(), &customers());
    assert_eq!(pairs(&rows), [(1, Some(10)), (1, Some(11)), (12, Some(12))]);
    let unmatched = rows[2].point.payload.as_ref().expect("test: payload");
    assert_eq!(unmatched["o"]["id"], serde_json::Value::Null);
    assert!(rows[2].score.abs() < f32::EPSILON);
}

#[test]
fn full_join_adds_both_unmatched_sides() {
    let sql = ON_HANDLE.replace("JOIN", "FULL JOIN");
    assert_eq!(
        pairs(&run(&sql, orders(), &customers())),
        [
            (1, Some(10)),
            (1, Some(11)),
            (2, None),
            (3, None),
            (12, Some(12))
        ]
    );
}

/// Two values match when a WHERE `=` finds them equal: `1` and `1.0` are one
/// number, `"1"` is not.
#[test]
fn keys_match_as_where_equality_does() {
    let rows = vec![order(1, json!({"cid": 1})), order(2, json!({"cid": "1"}))];
    let side = [customer(10, json!({"num": 1.0}))];
    let sql = "SELECT * FROM o JOIN c ON o.cid = c.num";
    assert_eq!(pairs(&run(sql, rows, &side)), [(1, Some(10))]);
}

/// `id` reads the point id on either side, and the equality orients by the
/// side that names the joined collection, whichever side of `ON` it is on.
#[test]
fn id_reads_the_point_id_and_on_orients_by_the_joined_side() {
    let rows = vec![order(1, json!({"cid": 10})), order(2, json!({"cid": 99}))];
    let sql = "SELECT * FROM o JOIN c ON c.id = o.cid";
    assert_eq!(pairs(&run(sql, rows, &customers())), [(1, Some(10))]);
}

#[test]
fn using_joins_the_column_both_sides_name() {
    let rows = vec![order(1, json!({"handle": "zoe"}))];
    let sql = "SELECT * FROM o JOIN c USING (handle)";
    assert_eq!(pairs(&run(sql, rows, &customers())), [(1, Some(12))]);
}

/// A self-join names the joined side by its alias, though the base side
/// carries the same table name.
#[test]
fn a_self_join_reads_its_alias_as_the_joined_side() {
    let join = join_of("SELECT * FROM emp JOIN emp AS m ON emp.boss = m.id");
    let keys = JoinKeys::of(&join).expect("test: keys");
    assert_eq!(keys.side, "id");
    assert_eq!(keys.base.table.as_deref(), Some("emp"));
    assert_eq!(keys.base.column, "boss");
}

#[test]
fn the_row_budget_bounds_the_output() {
    let join = join_of(&ON_HANDLE.replace("JOIN", "FULL JOIN"));
    let keys = JoinKeys::of(&join).expect("test: keys");
    let rows = hash_join(orders(), &customers(), &join, &keys, &["o"], 1);
    assert_eq!(pairs(&rows), [(1, Some(10))]);
}

#[test]
fn a_multi_column_using_has_no_join_keys() {
    let join = join_of("SELECT * FROM o JOIN c USING (a, b)");
    assert_eq!(JoinKeys::of(&join), None);
}
