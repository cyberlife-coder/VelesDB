//! Ordering of computed similarity scores and prefixes of a ranked result.

use crate::velesql::OrderByExpr;
use serde_json::json;

#[test]
fn window_alias_keeps_the_input_embedding() {
    use crate::point::Point;
    use crate::velesql::Parser;

    let (_dir, col) = crate::test_fixtures::fixtures::setup_collection(2);
    let points = [(1, [0.0, 1.0]), (2, [1.0, 0.0])]
        .into_iter()
        .map(|(id, emb)| {
            let payload = json!({ "emb": emb, "meta": { "emb": emb } });
            Point::new(id, vec![1.0, 0.0], Some(payload))
        });
    col.upsert(points).expect("test: seed embeddings");
    let params = std::collections::HashMap::from([("q".to_owned(), json!([1.0, 0.0]))]);
    for (prefix, alias, order, expected) in [
        ("", "rn", "similarity(emb, $q) DESC", 2),
        ("", "emb", "similarity(emb, $q) DESC", 2),
        ("", "meta", "similarity(meta.emb, $q) DESC", 2),
        ("", "emb", "emb ASC", 1),
        ("", "emb", "emb DESC", 2),
        ("", "meta", "(meta * 2) DESC", 2),
        ("LET k = emb * 2 ", "emb", "k DESC", 2),
    ] {
        let sql = format!(
            "{prefix}SELECT id, ROW_NUMBER() OVER (ORDER BY id) AS {alias} \
             FROM docs ORDER BY {order} LIMIT 1"
        );
        let query = Parser::parse(&sql).expect("test: parse window query");
        let rows = col
            .execute_query(&query, &params)
            .expect("test: order window");
        assert_eq!(rows.len(), 1, "{sql}");
        assert_eq!(rows[0].point.id, expected, "{sql}");
        assert_eq!(
            rows[0].point.payload.as_ref().unwrap()[alias],
            expected,
            "{sql}"
        );
        if order.starts_with("similarity(") {
            assert_eq!(rows[0].component_score("vector_score"), Some(1.0), "{sql}");
        }
    }
}

#[test]
fn similarity_score_order_keeps_nan_last_and_signed_zeros_tied() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    for descending in [true, false] {
        let key = crate::velesql::SelectOrderBy {
            expr: OrderByExpr::SimilarityBare,
            descending,
        };
        for higher in [true, false] {
            let cmp = |a, b| key.directed_score(a, b, higher);
            let numeric = if descending == higher { Greater } else { Less };
            assert_eq!(cmp(Some(-1.0), Some(1.0)), numeric);
            assert_eq!(cmp(Some(-0.0), Some(0.0)), Equal);
            for nan in [f32::NAN, -f32::NAN] {
                assert_eq!(cmp(Some(nan), Some(f32::INFINITY)), Greater);
                assert_eq!(cmp(Some(f32::NEG_INFINITY), Some(nan)), Less);
                assert_eq!(cmp(Some(nan), Some(-nan)), Equal);
                assert_eq!(
                    cmp(None, Some(nan)),
                    if descending { Less } else { Greater }
                );
            }
        }
    }
}

/// Ordering only the `leading` first rows puts exactly the rows a full
/// sort puts there, in the same order — ties broken by id, a missing key
/// last under `ASC` and first under `DESC` — for every `leading` from
/// none to past the end.
#[test]
fn test_leading_rows_equal_the_full_sort_prefix() {
    use crate::point::{Point, SearchResult};
    use crate::velesql::SelectOrderBy;

    let (_dir, col) = crate::test_fixtures::fixtures::setup_collection(2);
    let rows: Vec<SearchResult> = (1..=30u64)
        .map(|id| {
            // Four equal-key buckets in an order unrelated to the id,
            // and every seventh row without the key.
            let payload = if id.is_multiple_of(7) {
                json!({})
            } else {
                json!({ "g": (id * 5) % 4 })
            };
            SearchResult::new(Point::new(id, vec![1.0, 0.0], Some(payload)), 0.0)
        })
        .collect();
    let params = std::collections::HashMap::new();
    for descending in [false, true] {
        let order_by = [SelectOrderBy {
            expr: OrderByExpr::Field("g".to_string()),
            descending,
        }];
        let mut full = rows.clone();
        col.apply_order_by(&mut full, &order_by, &params)
            .expect("test: full sort");
        for leading in 0..=rows.len() + 1 {
            let mut partial = rows.clone();
            col.apply_order_by_with_let(&mut partial, &order_by, &params, &[], leading)
                .expect("test: leading sort");
            let kept = leading.min(rows.len());
            let ids = |r: &[SearchResult]| r[..kept].iter().map(|r| r.point.id).collect::<Vec<_>>();
            assert_eq!(
                ids(&partial),
                ids(&full),
                "descending {descending}, leading {leading}"
            );
        }
    }
}
