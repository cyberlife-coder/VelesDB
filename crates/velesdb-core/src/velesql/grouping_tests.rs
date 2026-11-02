//! Tests for [`super::grouping`]: the best row of a group and the group order
//! never depend on the order rows arrive in, numbers are one group key by
//! value, HAVING binds `AND` tighter than `OR`, and the group budget.

use serde_json::{json, Value as Json};

use super::ast::{WithClause, WithValue};
use super::grouping::{max_groups, DEFAULT_MAX_GROUPS, SERVER_MAX_GROUPS_CEILING};
use super::{GroupInput, GroupOutput, Grouper, Parser, SelectStatement};

fn select(sql: &str) -> SelectStatement {
    Parser::parse(sql).expect("test: parse").select
}

/// Groups `rows` (`id`, `score`, payload) for `sql`, in the order given.
fn group(sql: &str, rows: &[(u64, f32, Json)], higher_is_better: bool) -> Vec<GroupOutput> {
    let stmt = select(sql);
    let mut grouper =
        Grouper::new(&stmt, stmt.having.as_ref(), higher_is_better).expect("test: grouper");
    for (id, score, payload) in rows {
        grouper
            .push(GroupInput::new(*id, *score, Some(payload)))
            .expect("test: push");
    }
    grouper.finish()
}

fn rows_of(groups: &[GroupOutput]) -> Vec<Json> {
    groups.iter().map(|g| g.row.clone()).collect()
}

#[test]
fn test_first_reads_the_smallest_id_whatever_the_feed_order_in_a_sql_group_by() {
    let sql = "SELECT c, FIRST(t) AS t FROM docs GROUP BY c";
    let rows = [
        (7, 0.0, json!({"c": "a", "t": "seven"})),
        (2, 0.0, json!({"c": "a", "t": "two"})),
        (5, 0.0, json!({"c": "a", "t": "five"})),
    ];
    let mut reversed = rows.clone();
    reversed.reverse();
    for feed in [&rows[..], &reversed[..]] {
        let groups = group(sql, feed, true);
        assert_eq!(rows_of(&groups), vec![json!({"c": "a", "t": "two"})]);
        assert_eq!(groups[0].id, 2);
    }
}

#[test]
fn test_a_vector_group_keeps_its_most_similar_row_and_breaks_score_ties_by_id() {
    let sql = "SELECT c, FIRST(t) AS t FROM docs WHERE vector NEAR $v GROUP BY c";
    let rows = [
        (9, 0.5, json!({"c": "a", "t": "nine"})),
        (4, 0.9, json!({"c": "a", "t": "four"})),
        (3, 0.9, json!({"c": "a", "t": "three"})),
        (1, 0.2, json!({"c": "a", "t": "one"})),
    ];
    let mut reversed = rows.clone();
    reversed.reverse();
    for feed in [&rows[..], &reversed[..]] {
        let groups = group(sql, feed, true);
        assert_eq!(groups[0].id, 3, "0.9 is best; 3 and 4 tie, 3 is smaller");
        assert_eq!(groups[0].row["t"], json!("three"));
    }
    // A distance metric: the lowest score is the most similar.
    let groups = group(sql, &rows, false);
    assert_eq!(groups[0].id, 1);
}

#[test]
fn test_numbers_equal_by_value_are_one_group_key() {
    let sql = "SELECT k, COUNT(*) AS n FROM docs GROUP BY k";
    let rows = [
        (1, 0.0, json!({"k": 1})),
        (2, 0.0, json!({"k": 1.0})),
        (3, 0.0, json!({"k": 0.0})),
        (4, 0.0, json!({"k": -0.0})),
        (5, 0.0, json!({"k": 0})),
        (6, 0.0, json!({"k": 1.5})),
    ];
    assert_eq!(
        rows_of(&group(sql, &rows, true)),
        vec![
            json!({"k": 0, "n": 3}),
            json!({"k": 1, "n": 2}),
            json!({"k": 1.5, "n": 1}),
        ]
    );
}

#[test]
fn test_an_integer_and_a_float_of_one_value_are_one_group_beyond_f64_precision() {
    let sql = "SELECT k, COUNT(*) AS n FROM docs GROUP BY k";
    let rows = [
        (1, 0.0, json!({"k": 9_007_199_254_740_992_i64})),
        (2, 0.0, json!({"k": 9_007_199_254_740_992.0_f64})),
        (3, 0.0, json!({"k": 9_007_199_254_740_993_i64})),
        (4, 0.0, json!({"k": 9_223_372_036_854_775_808.0_f64})),
        (5, 0.0, json!({"k": 9_223_372_036_854_775_808_u64})),
    ];
    assert_eq!(
        rows_of(&group(sql, &rows, true)),
        vec![
            json!({"k": 9_007_199_254_740_992_i64, "n": 2}),
            json!({"k": 9_007_199_254_740_993_i64, "n": 1}),
            json!({"k": 9_223_372_036_854_775_808_u64, "n": 2}),
        ]
    );
}

#[test]
fn test_groups_whose_keys_compare_equal_come_by_best_id_whatever_the_feed_order() {
    // Two arrays compare equal in ORDER BY's value order: only the best row's
    // id can order their groups.
    let sql = "SELECT k, COUNT(*) AS n FROM docs GROUP BY k";
    let rows = [
        (4, 0.0, json!({"k": [2]})),
        (1, 0.0, json!({"k": [1]})),
        (3, 0.0, json!({"k": [3]})),
    ];
    let mut reversed = rows.clone();
    reversed.reverse();
    for feed in [&rows[..], &reversed[..]] {
        let ids: Vec<u64> = group(sql, feed, true).iter().map(|g| g.id).collect();
        assert_eq!(ids, vec![1, 3, 4]);
    }
}

#[test]
fn test_a_null_having_threshold_matches_no_group_as_sql_unknown() {
    let stmt = select("SELECT k, COUNT(*) AS n FROM docs GROUP BY k HAVING COUNT(*) > $n");
    let mut having = stmt.having.clone().expect("test: having");
    having.conditions[0].value = super::Value::Null;
    let mut grouper = Grouper::new(&stmt, Some(&having), true).expect("test: NULL is no error");
    let payload = json!({"k": "a"});
    grouper
        .push(GroupInput::new(1, 0.0, Some(&payload)))
        .expect("test: push");
    assert!(grouper.finish().is_empty());
}

#[test]
fn test_sql_groups_come_by_key_ascending_with_null_last() {
    let sql = "SELECT k, COUNT(*) AS n FROM docs GROUP BY k";
    let rows = [
        (1, 0.0, json!({"k": "b"})),
        (2, 0.0, json!({})),
        (3, 0.0, json!({"k": "a"})),
    ];
    assert_eq!(
        rows_of(&group(sql, &rows, true)),
        vec![
            json!({"k": "a", "n": 1}),
            json!({"k": "b", "n": 1}),
            json!({"k": null, "n": 1}),
        ]
    );
}

#[test]
fn test_having_and_binds_tighter_than_or() {
    // n = 2 OR (n > 0 AND n > 5): the group of two passes on the left alone.
    let sql = "SELECT k, COUNT(*) AS n FROM docs GROUP BY k \
               HAVING COUNT(*) = 2 OR COUNT(*) > 0 AND COUNT(*) > 5";
    let rows = [
        (1, 0.0, json!({"k": "a"})),
        (2, 0.0, json!({"k": "a"})),
        (3, 0.0, json!({"k": "b"})),
    ];
    assert_eq!(
        rows_of(&group(sql, &rows, true)),
        vec![json!({"k": "a", "n": 2})]
    );
}

#[test]
fn test_a_non_numeric_bound_having_threshold_is_refused_with_v018() {
    let stmt = select("SELECT k, COUNT(*) AS n FROM docs GROUP BY k HAVING COUNT(*) > $n");
    let mut having = stmt.having.clone().expect("test: having");
    having.conditions[0].value = super::Value::String("many".to_string());
    let err = Grouper::new(&stmt, Some(&having), true).expect_err("test: refused");
    assert!(err.to_string().contains("[V018]"), "got: {err}");
    assert!(
        err.to_string().contains("\"many\""),
        "the JSON value: {err}"
    );
}

#[test]
fn test_a_sql_group_by_refuses_a_group_past_its_budget() {
    let stmt = select("SELECT k, COUNT(*) AS n FROM docs GROUP BY k WITH (max_groups = 1)");
    let mut grouper = Grouper::new(&stmt, None, true).expect("test: grouper");
    let (a, b) = (json!({"k": "a"}), json!({"k": "b"}));
    grouper
        .push(GroupInput::new(1, 0.0, Some(&a)))
        .expect("test: first group");
    grouper
        .push(GroupInput::new(2, 0.0, Some(&a)))
        .expect("test: same group");
    let err = grouper
        .push(GroupInput::new(3, 0.0, Some(&b)))
        .expect_err("test: second group");
    assert!(
        err.to_string().contains("Too many groups (limit: 1)"),
        "got: {err}"
    );
}

// -------------------------------------------------------------------------
// #903: the GROUP BY group budget is capped server-side.
// -------------------------------------------------------------------------

#[test]
fn test_max_groups_is_clamped_to_the_server_ceiling() {
    let with = WithClause::new().with_option("max_groups", WithValue::Integer(i64::MAX));
    assert_eq!(max_groups(Some(&with)), SERVER_MAX_GROUPS_CEILING);
}

#[test]
fn test_max_groups_below_the_ceiling_is_honoured() {
    let with = WithClause::new().with_option("group_limit", WithValue::Integer(42));
    assert_eq!(max_groups(Some(&with)), 42);
}

#[test]
fn test_max_groups_defaults_well_below_the_ceiling() {
    assert_eq!(max_groups(None), DEFAULT_MAX_GROUPS);
    assert!(DEFAULT_MAX_GROUPS < SERVER_MAX_GROUPS_CEILING);
}
