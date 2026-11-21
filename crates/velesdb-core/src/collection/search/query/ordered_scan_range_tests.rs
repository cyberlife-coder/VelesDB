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
