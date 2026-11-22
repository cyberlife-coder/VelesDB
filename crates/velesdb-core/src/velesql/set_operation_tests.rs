use super::*;

use serde_json::json;

use crate::point::Point;
use crate::velesql::{OrderByExpr, Parser};

fn row(id: u64, score: f32, payload: serde_json::Value) -> SearchResult {
    SearchResult::new(Point::metadata_only(id, payload), score)
}

fn parse(sql: &str) -> Query {
    Parser::parse(sql).expect("test: parse")
}

fn ids(rows: &[SearchResult]) -> Vec<u64> {
    rows.iter().map(|r| r.point.id).collect()
}

/// Runs `sql`'s compound over fixed operand rows, ordering by `id` ascending
/// when the window has an ORDER BY.
fn run(sql: &str, operands: Vec<Vec<SearchResult>>, higher_is_better: bool) -> Vec<SearchResult> {
    let query = parse(sql);
    let mut operands = operands.into_iter();
    execute_compound::<()>(
        &query,
        higher_is_better,
        100_000,
        |_| Ok(operands.next().expect("test: one row set per operand")),
        |rows, _| {
            rows.sort_by_key(|r| r.point.id);
            Ok(())
        },
    )
    .expect("test: compound")
}

#[test]
fn the_last_operands_clauses_window_the_combined_rows() {
    let query =
        parse("SELECT * FROM a LIMIT 3 UNION SELECT * FROM b ORDER BY year ASC LIMIT 2 OFFSET 1");
    let window = compound_window(&query);
    assert_eq!((window.offset, window.limit), (1, Some(2)));
    assert!(window.order_by.is_some());

    let operands = operand_queries(&query, 1000);
    assert_eq!(
        operands[0].select.limit,
        Some(3),
        "a first operand keeps its LIMIT"
    );
    let last = &operands[1].select;
    assert_eq!(
        (last.order_by.is_none(), last.offset, last.limit),
        (true, None, Some(1000)),
        "the last operand runs unwindowed, up to the ceiling"
    );
}

/// A compound has no implicit LIMIT: every combined row, up to the ceiling.
#[test]
fn without_a_limit_every_combined_row_comes_back() {
    let query = parse("SELECT * FROM a UNION SELECT * FROM b");
    assert_eq!(compound_window(&query).limit, None);
    let rows: Vec<SearchResult> = (1..=12).map(|id| row(id, 1.0, json!({}))).collect();
    let combined = run(
        "SELECT * FROM a UNION SELECT * FROM b",
        vec![rows, Vec::new()],
        true,
    );
    assert_eq!(combined.len(), 12, "not the SELECT default of 10");
}

/// `*` keys by id and payload: the same point is one row, two points with
/// one payload are two.
#[test]
fn union_returns_each_projected_row_once() {
    let rows = run(
        "SELECT * FROM a UNION SELECT * FROM b",
        vec![
            vec![row(1, 0.5, json!({"k": 1})), row(2, 0.5, json!({"k": 1}))],
            vec![row(1, 0.9, json!({"k": 1})), row(3, 0.1, json!({"k": 2}))],
        ],
        true,
    );
    assert_eq!(ids(&rows), vec![1, 2, 3]);
    assert!(
        (rows[0].score - 0.9).abs() < f32::EPSILON,
        "the better score"
    );
}

/// Listed columns key by value, by position: the spec's
/// `SELECT item_id … INTERSECT SELECT item_id …`.
#[test]
fn intersect_compares_the_projected_values() {
    let rows = run(
        "SELECT item_id FROM likes WHERE u = 1 INTERSECT SELECT item_id FROM likes WHERE u = 2",
        vec![
            vec![
                row(10, 1.0, json!({"item_id": 7})),
                row(11, 1.0, json!({"item_id": 8})),
            ],
            vec![
                row(20, 1.0, json!({"item_id": 8})),
                row(21, 1.0, json!({"item_id": 9})),
            ],
        ],
        true,
    );
    assert_eq!(ids(&rows), vec![11]);
}

#[test]
fn except_keeps_the_distinct_left_rows_the_right_lacks() {
    let rows = run(
        "SELECT id FROM all_items EXCEPT SELECT id FROM purchased",
        vec![
            vec![
                row(1, 1.0, json!({})),
                row(2, 1.0, json!({})),
                row(2, 1.0, json!({})),
            ],
            vec![row(1, 1.0, json!({}))],
        ],
        true,
    );
    assert_eq!(ids(&rows), vec![2]);
}

#[test]
fn union_all_keeps_every_row() {
    let rows = run(
        "SELECT * FROM a UNION ALL SELECT * FROM a",
        vec![vec![row(1, 1.0, json!({}))], vec![row(1, 1.0, json!({}))]],
        true,
    );
    assert_eq!(ids(&rows), vec![1, 1]);
}

/// Without ORDER BY the rows rank by score in the metric's direction, ties in
/// the order met; a distance ranks closest first.
#[test]
fn rows_rank_by_score_in_the_metrics_direction() {
    let operands = || {
        vec![
            vec![row(1, 2.0, json!({})), row(2, 1.0, json!({}))],
            vec![row(3, 1.0, json!({})), row(4, 3.0, json!({}))],
        ]
    };
    let sql = "SELECT * FROM a UNION SELECT * FROM b";
    assert_eq!(ids(&run(sql, operands(), true)), vec![4, 1, 2, 3]);
    assert_eq!(ids(&run(sql, operands(), false)), vec![2, 3, 1, 4]);
}

#[test]
fn the_windows_order_by_sorts_the_combined_rows() {
    let rows = run(
        "SELECT * FROM a UNION SELECT * FROM b ORDER BY id ASC LIMIT 2",
        vec![
            vec![row(3, 1.0, json!({}))],
            vec![row(1, 1.0, json!({})), row(2, 1.0, json!({}))],
        ],
        true,
    );
    assert_eq!(ids(&rows), vec![1, 2]);
}

/// An operand's rows come out of the SELECT post-processing, its window
/// functions evaluated: their outputs are projected values, part of the
/// row's identity.
#[test]
fn window_outputs_are_part_of_a_rows_identity() {
    let sql = "SELECT category, ROW_NUMBER() OVER (ORDER BY id ASC) AS rn FROM a \
               UNION SELECT category, ROW_NUMBER() OVER (ORDER BY id ASC) AS rn FROM a";
    let numbered = || {
        vec![
            row(1, 1.0, json!({"category": "tech", "rn": 1})),
            row(2, 1.0, json!({"category": "tech", "rn": 2})),
        ]
    };
    let rows = run(sql, vec![numbered(), numbered()], true);
    assert_eq!(ids(&rows), [1, 2], "(tech, 1) and (tech, 2) are two rows");
}

/// A duplicate whose score wins brings its score breakdown along, so
/// `vector_score` reads the score the row now carries.
#[test]
fn the_better_duplicate_brings_its_component_scores() {
    let scored = |id: u64, score: f32| {
        SearchResult::with_component_scores(
            Point::metadata_only(id, json!({})),
            score,
            smallvec::smallvec![("vector_score", score)],
        )
    };
    for operator in ["UNION", "INTERSECT"] {
        let rows = run(
            &format!("SELECT * FROM a {operator} SELECT * FROM a"),
            vec![vec![scored(1, 0.2)], vec![scored(1, 0.9)]],
            true,
        );
        let kept = &rows[0];
        assert!(
            (kept.score - 0.9).abs() < f32::EPSILON,
            "{operator}: {}",
            kept.score
        );
        assert_eq!(
            kept.component_scores.as_deref(),
            Some(&[("vector_score", 0.9)][..]),
            "{operator}"
        );
    }
}

/// An explicit LIMIT above the engine's ceiling is held to it, as a SELECT's
/// is.
#[test]
fn an_explicit_limit_is_held_to_the_ceiling() {
    let query = parse("SELECT * FROM a UNION ALL SELECT * FROM b LIMIT 200000");
    let rows: Vec<SearchResult> = (1..=6).map(|id| row(id, 1.0, json!({}))).collect();
    let mut operands = vec![rows.clone(), rows].into_iter();
    let combined = execute_compound::<()>(
        &query,
        true,
        4,
        |_| Ok(operands.next().expect("test: operand")),
        |_, _| Ok(()),
    )
    .expect("test: compound");
    assert_eq!(combined.len(), 4);
}

/// A compound's columns are positional, named by the first operand: a later
/// operand's value lands under the first operand's column, so the first
/// operand's projection and ORDER BY read it.
#[test]
fn a_later_operands_values_take_the_first_operands_column_names() {
    let rows = run(
        "SELECT year FROM a UNION ALL SELECT age FROM b",
        vec![
            vec![row(1, 1.0, json!({"year": 2020}))],
            vec![row(2, 1.0, json!({"age": 30, "year": 1999}))],
        ],
        true,
    );
    let years: Vec<&serde_json::Value> = rows
        .iter()
        .map(|r| &r.point.payload.as_ref().expect("test: payload")["year"])
        .collect();
    assert_eq!(years, [&json!(2020), &json!(30)]);
}

#[test]
fn vector_sort_slots_preserve_user_keys_on_rows_sharing_an_id() {
    let query = parse(
        r#"SELECT a AS meta, a AS dup FROM docs UNION ALL SELECT b, c FROM other
           ORDER BY similarity(meta.vec, [1, 0]) DESC, dup ASC, ".2" ASC"#,
    );
    let first = json!({"a": 3, "meta": {"vec": [0, 1]}, ".0": "first", ".1": null});
    let second = json!({"b": 4, "c": 8, "meta": {"vec": [1, 0]}, ".0": "second", ".1": null});
    let mut calls = [vec![row(7, 0.0, first)], vec![row(7, 0.0, second)]].into_iter();
    let ordered = execute_compound::<()>(
        &query,
        true,
        100,
        |_| Ok(calls.next().expect("one row set per operand")),
        |rows, order| {
            let OrderByExpr::Field(scalar) = &order[1].expr else {
                panic!("scalar key")
            };
            let values: Vec<_> = rows
                .iter()
                .map(|row| {
                    let payload = row.point.payload.as_ref().expect("a payload");
                    assert!(crate::velesql::row_field(payload, ".2").is_none());
                    (payload["meta"]["vec"].clone(), payload[scalar].clone())
                })
                .collect();
            assert_eq!(
                values,
                [(json!([0, 1]), json!(3)), (json!([1, 0]), json!(8))]
            );
            rows.reverse();
            Ok(())
        },
    )
    .expect("compound");
    let payloads: Vec<_> = ordered.into_iter().map(|row| row.point.payload).collect();
    assert_eq!(
        payloads,
        [
            Some(json!({"a": 4, "b": 4, "c": 8, "meta": 8, ".0": "second", ".1": null})),
            Some(json!({"a": 3, "meta": 3, ".0": "first", ".1": null})),
        ]
    );
}

#[test]
fn a_vector_sort_preserves_a_window_name_from_an_empty_first_operand() {
    let query = r#"SELECT a AS meta, a AS dup, RANK() OVER (ORDER BY a) AS ".0" FROM docs
        UNION ALL SELECT b, c, RANK() OVER (ORDER BY c) AS rn FROM other
        ORDER BY similarity(meta.vec, [1, 0]) DESC"#;
    let payload = json!({"b": 7, "c": 8, "rn": 1, "meta": {"vec": [1, 0]}});
    let rows = run(query, vec![vec![], vec![row(7, 0.0, payload)]], true);
    let actual = rows[0].point.payload.as_ref().expect("a payload");
    assert_eq!(actual[".0"], 1);
    assert_eq!(actual["meta"], 8);
}

/// A NaN score ranks after every number in both directions, whatever its
/// sign, and never displaces a duplicate's number.
#[test]
fn a_nan_score_ranks_last_and_never_wins_a_duplicate() {
    for nan in [f32::NAN, -f32::NAN] {
        for higher_is_better in [true, false] {
            let case = format!(
                "nan sign {}, higher_is_better {higher_is_better}",
                nan.is_sign_negative()
            );
            let ranked = run(
                "SELECT * FROM a UNION SELECT * FROM b",
                vec![
                    vec![row(1, nan, json!({}))],
                    vec![row(2, 0.5, json!({})), row(3, 0.7, json!({}))],
                ],
                higher_is_better,
            );
            assert_eq!(ids(&ranked).last(), Some(&1), "{case}");

            let merged = run(
                "SELECT * FROM a UNION SELECT * FROM a",
                vec![vec![row(1, nan, json!({}))], vec![row(1, 0.5, json!({}))]],
                higher_is_better,
            );
            assert!(
                (merged[0].score - 0.5).abs() < f32::EPSILON,
                "{case}: {}",
                merged[0].score
            );
        }
    }
}

#[test]
fn comparing_wildcard_rows_preserves_their_payload_member_order() {
    let payload: serde_json::Value = serde_json::from_str(
        r#"{"start":1,"id":99,"middle":2,"other":3,"s":"original","inside":4,"ending":5,"zz":6,"r":1}"#,
    )
    .unwrap();
    let original = payload.to_string();
    let sql = "SELECT t.*, similarity() AS s, RANK() OVER (ORDER BY id) AS r FROM a AS t WHERE vector NEAR [1,0] UNION SELECT t.*, similarity() AS s, RANK() OVER (ORDER BY id) AS r FROM b AS t WHERE vector NEAR [1,0]";
    let rows = run(
        sql,
        vec![
            vec![row(7, 1.0, payload.clone())],
            vec![row(7, 1.0, payload)],
        ],
        true,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].point.payload.as_ref().unwrap().to_string(),
        original
    );
}
