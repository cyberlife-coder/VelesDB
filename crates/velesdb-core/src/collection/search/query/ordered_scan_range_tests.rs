use super::{graph, ids, planned_ids, Collection};
use serde_json::{json, Value};

fn rows(entries: &[(u64, Value)]) -> (tempfile::TempDir, Collection) {
    let (dir, col) = graph(None);
    let entries: Vec<_> = entries.iter().map(|(id, value)| (*id, value)).collect();
    col.store_node_payloads(&entries)
        .expect("test: write nodes");
    (dir, col)
}

#[test]
fn ranked_window_uses_the_range_candidates_before_applying_limit() {
    let (_dir, col) = rows(&[
        (1, json!({"rank": 1, "price": 1, "cat": "x"})),
        (2, json!({"rank": 8, "price": 9, "cat": "x"})),
        (3, json!({"rank": 4, "price": 2, "cat": "y"})),
        (4, json!({"rank": 9, "price": 3, "cat": "x"})),
    ]);
    let predicate = "rank BETWEEN 8 AND 9";
    let sql = format!("SELECT * FROM rows WHERE {predicate} ORDER BY price LIMIT 1");
    assert_eq!(planned_ids(&col, predicate).unwrap(), [1, 2, 3, 4]);
    assert_eq!(ids(&col, &sql), [4]);
    col.create_index("rank").expect("test: range index");
    col.create_index("cat").expect("test: equality index");
    assert_eq!(planned_ids(&col, predicate).unwrap(), [2, 4]);
    assert_eq!(ids(&col, &sql), [4]);
    assert_eq!(
        planned_ids(&col, "rank >= 8 AND cat = 'x'").unwrap(),
        [2, 4]
    );
    assert_eq!(
        planned_ids(&col, "rank >= 8 OR rank < 2").unwrap(),
        [1, 2, 4]
    );
    assert!(planned_ids(&col, "rank > 9").unwrap().is_empty());
    let uncovered = "rank >= 8 OR price = 2";
    assert_eq!(planned_ids(&col, uncovered).unwrap(), [1, 2, 3, 4]);
    assert_eq!(
        ids(
            &col,
            &format!("SELECT * FROM rows WHERE {uncovered} ORDER BY price LIMIT 1")
        ),
        [3]
    );
}

#[test]
fn incomplete_range_bitmaps_keep_large_ids_and_exact_bounds() {
    let large_id = u64::from(u32::MAX) + 1;
    let boundary = 1_u64 << 53;
    let (_dir, col) = rows(&[
        (1, json!({"rank": 0, "price": 2})),
        (2, json!({"rank": boundary + 1, "price": 3})),
        (large_id, json!({"rank": boundary, "price": 1})),
    ]);
    col.create_index("rank").expect("test: range index");
    let all = [1, 2, large_id];
    assert_eq!(planned_ids(&col, "rank >= 1").unwrap(), all);
    assert_eq!(
        ids(
            &col,
            "SELECT * FROM rows WHERE rank >= 1 ORDER BY price LIMIT 1"
        ),
        [large_id]
    );
    let strict = format!("rank > {boundary}");
    assert_eq!(planned_ids(&col, &strict).unwrap(), all);
    assert_eq!(
        ids(
            &col,
            &format!("SELECT * FROM rows WHERE {strict} ORDER BY price LIMIT 1")
        ),
        [2]
    );
    assert_eq!(
        planned_ids(&col, &format!("id = {large_id}")).unwrap(),
        [large_id]
    );
}

#[test]
fn a_literal_dotted_index_does_not_answer_a_nested_predicate() {
    let (_dir, col) = rows(&[
        (1, json!({"meta.rank": 8, "meta": {"rank": 1}, "price": 1})),
        (2, json!({"meta.rank": 1, "meta": {"rank": 8}, "price": 2})),
    ]);
    let check = || {
        for predicate in [
            "meta.rank = 8",
            "meta.rank IN (8)",
            "meta.rank >= 8",
            "meta.rank BETWEEN 8 AND 9",
        ] {
            let sql = format!("SELECT * FROM rows WHERE {predicate} ORDER BY price LIMIT 1");
            assert_eq!(ids(&col, &sql), [2], "{predicate}");
            assert_eq!(planned_ids(&col, predicate).unwrap(), [1, 2], "{predicate}");
        }
    };
    check();
    col.create_index("meta.rank")
        .expect("test: literal dotted index");
    check();
}

#[test]
fn an_indexed_between_reads_only_its_inclusive_interval() {
    let large = u64::from(u32::MAX) + 1;
    let (_dir, col) = rows(&[
        (large, json!({"rank": 1, "category": "edge"})),
        (2, json!({"rank": 5, "category": "middle"})),
        (large + 1, json!({"rank": 9, "category": "edge"})),
    ]);
    col.create_index("rank").expect("test: rank index");
    col.create_index("category").expect("test: category index");
    let between = "rank BETWEEN 5 AND 5";
    // Both one-sided lookups are incomplete because each sees a large id.
    // The joint interval contains just the representable id.
    for predicate in [
        between.to_owned(),
        format!("({between}) AND category = 'middle'"),
        format!("({between}) AND id IN (2, {large})"),
    ] {
        assert_eq!(planned_ids(&col, &predicate).unwrap(), [2], "{predicate}");
    }
    assert!(planned_ids(&col, "rank BETWEEN 6 AND 3")
        .unwrap()
        .is_empty());
    let sql = "SELECT * FROM rows WHERE rank BETWEEN 1 AND 5 ORDER BY id";
    assert_eq!(
        ids(&col, sql),
        [2, large],
        "an in-range large ID still counts"
    );
}

#[test]
fn an_ordered_time_window_uses_execution_values_in_its_index_and_filter() {
    use crate::velesql::{Condition, Parser, Value as SqlValue};
    let (_dir, col) = rows(&[]);
    let syntax = Parser::parse(
        "SELECT * FROM rows WHERE ts BETWEEN NOW() - INTERVAL '1 second' AND NOW() ORDER BY price",
    )
    .unwrap();
    let prepared = col
        .bind_select(&syntax, &std::collections::HashMap::new())
        .unwrap()
        .unwrap();
    let Some(Condition::Between(bounds)) = prepared.select.where_clause else {
        panic!("test: missing execution bounds");
    };
    let (SqlValue::Integer(low), SqlValue::Integer(high)) = (bounds.low, bounds.high) else {
        panic!("both index lookup and row validation need concrete execution bounds");
    };
    assert_eq!(low, high - 1);
    let Some(Condition::Between(original)) = syntax.select.where_clause else {
        panic!("test: missing source bounds");
    };
    assert!(matches!(original.high, SqlValue::Temporal(_)));
}

#[test]
fn required_equalities_bound_the_work_of_an_ordered_range() {
    let (_dir, col) = rows(&[
        (1, json!({"rank": 1, "cat": "rare"})),
        (2, json!({"rank": 0, "cat": "rare"})),
        (3, json!({"rank": 2, "cat": "common"})),
        (4, json!({"rank": 3, "cat": "common"})),
        (5, json!({"rank": 4, "cat": "common"})),
    ]);
    col.create_index("rank").unwrap();
    col.create_index("cat").unwrap();
    for required in ["cat = 'rare'", "id IN (1, 2)"] {
        let predicate = format!("({required}) AND rank BETWEEN 1 AND 4");
        assert_eq!(planned_ids(&col, &predicate).unwrap(), [1, 2], "{required}");
        let query = format!("SELECT * FROM rows WHERE {predicate} ORDER BY id");
        assert_eq!(ids(&col, &query), [1], "{required}");
    }
}

#[test]
fn wide_integer_interval_bounds_and_partial_or_keep_all_sql_matches() {
    let wide = (1_i64 << 53) + 1;
    let (_dir, col) = rows(&[
        (1, json!({"rank": -wide, "label": "other"})),
        (2, json!({"rank": 1 - wide, "label": "edge"})),
        (3, json!({"rank": wide - 1, "label": "edge"})),
        (4, json!({"rank": wide, "label": "other"})),
        (5, json!({"rank": u64::MAX, "label": "other"})),
        (6, json!({"rank": i64::MIN, "label": "other"})),
    ]);
    col.create_index("rank").unwrap();
    for (value, id) in [
        ((-wide).to_string(), 1),
        (wide.to_string(), 4),
        (u64::MAX.to_string(), 5),
        (i64::MIN.to_string(), 6),
    ] {
        let predicate = format!("rank BETWEEN {value} AND {value}");
        let sql = format!("SELECT * FROM rows WHERE {predicate} ORDER BY id");
        assert_eq!(planned_ids(&col, &predicate).unwrap(), [1, 2, 3, 4, 5, 6]);
        assert_eq!(ids(&col, &sql), [id], "{predicate}");
    }
    let partial = "rank BETWEEN -1 AND 1 OR label = 'edge'";
    assert_eq!(planned_ids(&col, partial).unwrap(), [1, 2, 3, 4, 5, 6]);
    assert_eq!(
        ids(
            &col,
            &format!("SELECT * FROM rows WHERE {partial} ORDER BY id")
        ),
        [2, 3]
    );
}
