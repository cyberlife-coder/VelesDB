//! An expired row is invisible on every read surface: it takes no place in
//! a search's `k`, nor in a hybrid branch's, and a search for `k` rows
//! returns `k` live ones while the collection holds them.

use std::collections::{BTreeMap, HashSet};

use serde_json::{json, Value};

use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::index::sparse::SparseVector;
use crate::point::{Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn expired(payload: Value) -> Value {
    let mut payload = payload;
    payload["_veles_expires_at"] = json!(1);
    payload
}

/// 1 and 2, expired, are the best BM25 and sparse hits and far from the
/// query `[1, 0]`; 10 is the second nearest and the best live hit; 11 is the
/// nearest, with no hit; 12 neither.
fn hybrid(dir: &std::path::Path) -> Collection {
    let collection =
        Collection::create(dir.join("h"), 2, DistanceMetric::Cosine).expect("test: collection");
    let sparse = |weight: f32| {
        Some(BTreeMap::from([(
            String::new(),
            SparseVector::new(vec![(7, weight)]),
        )]))
    };
    collection
        .upsert(vec![
            Point::with_sparse(
                1,
                vec![0.0, 1.0],
                Some(expired(json!({"content": "apple apple apple apple"}))),
                sparse(9.0),
            ),
            Point::with_sparse(
                2,
                vec![0.1, 1.0],
                Some(expired(json!({"content": "apple apple apple apple"}))),
                sparse(8.0),
            ),
            Point::with_sparse(
                10,
                vec![0.9, 0.3],
                Some(json!({"content": "apple apple apple zzz"})),
                sparse(5.0),
            ),
            Point::with_sparse(
                11,
                vec![1.0, 0.0],
                Some(json!({"content": "xx yy zz ww"})),
                None,
            ),
            Point::with_sparse(
                12,
                vec![0.5, 0.5],
                Some(json!({"content": "qq rr ss tt"})),
                None,
            ),
        ])
        .expect("test: upsert");
    collection
}

#[test]
fn an_expired_row_takes_no_place_in_a_hybrid_branch() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = hybrid(dir.path());

    let best: Vec<u64> = collection
        .bm25_rows("apple", 2, RowUniverse::Vectors)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(best, [10]);
    let hits = collection
        .hybrid_search(&[1.0, 0.0], "apple", 1, None, None)
        .expect("test: hybrid search");
    assert_eq!(ids(&hits), [10]);
}

/// 1..=6, expired, are the nearest vectors and the best text; 10..=13 live
/// ones further away.
fn nearest_expired(dir: &std::path::Path) -> Collection {
    let collection =
        Collection::create(dir.join("n"), 2, DistanceMetric::Cosine).expect("test: collection");
    collection
        .upsert((1..=6u64).map(|id| {
            #[allow(clippy::cast_precision_loss)]
            let vector = vec![1.0, 0.001 * id as f32];
            Point::new(
                id,
                vector,
                Some(expired(json!({"content": "apple apple apple apple"}))),
            )
        }))
        .expect("test: upsert expired");
    collection
        .upsert((10..=13u64).map(|id| {
            #[allow(clippy::cast_precision_loss)]
            let vector = vec![0.2, 1.0 + 0.01 * id as f32];
            Point::new(id, vector, Some(json!({"content": "apple zz yy xx"})))
        }))
        .expect("test: upsert live");
    collection
}

#[test]
fn a_search_returns_k_live_rows_past_the_expired_nearest() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = nearest_expired(dir.path());
    let live: HashSet<u64> = (10..=13).collect();

    for k in [1, 2, 3] {
        for hits in [
            collection.search(&[1.0, 0.0], k).expect("test: search"),
            collection
                .search_with_quality(&[1.0, 0.0], k, crate::SearchQuality::Balanced)
                .expect("test: search with quality"),
            collection
                .hybrid_search(&[1.0, 0.0], "apple", k, None, None)
                .expect("test: hybrid search"),
        ] {
            assert_eq!(hits.len(), k, "k = {k}: {:?}", ids(&hits));
            assert!(
                ids(&hits).iter().all(|id| live.contains(id)),
                "k = {k}: {:?}",
                ids(&hits)
            );
        }
    }
}

/// `rows` rows, every odd one expired, `cat` `a` for ids below 4 and `b`
/// above, all holding "apple".
fn half_expired(dir: &std::path::Path, rows: u64) -> Collection {
    let collection =
        Collection::create(dir.join("a"), 2, DistanceMetric::Cosine).expect("test: collection");
    collection
        .upsert((1..=rows).map(|id| {
            let cat = if id < 4 { "a" } else { "b" };
            let payload = json!({"k": 1, "cat": cat, "content": "apple"});
            let payload = if id % 2 == 1 {
                expired(payload)
            } else {
                payload
            };
            Point::new(id, vec![1.0, 0.5], Some(payload))
        }))
        .expect("test: upsert");
    collection
}

fn aggregate(collection: &Collection, sql: &str) -> Value {
    let query = crate::velesql::Parser::parse(sql).expect("test: parse");
    collection
        .execute_aggregate(&query, &std::collections::HashMap::new())
        .expect("test: aggregate")
}

/// An aggregate counts the rows a SELECT returns: an expired one in none,
/// whatever its WHERE and GROUP BY.
#[test]
fn an_aggregate_counts_no_expired_row() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = half_expired(dir.path(), 8);

    assert_eq!(
        aggregate(&collection, "SELECT COUNT(*) FROM a"),
        json!({"count": 4})
    );
    assert_eq!(
        aggregate(
            &collection,
            "SELECT COUNT(*) FROM a WHERE content MATCH 'apple'"
        ),
        json!({"count": 4})
    );
    assert_eq!(
        aggregate(
            &collection,
            "SELECT cat, COUNT(*) FROM a GROUP BY cat ORDER BY cat"
        ),
        json!([{"cat": "a", "count": 1}, {"cat": "b", "count": 3}])
    );
}

/// The parallel scan of a large aggregate skips expired rows too.
#[test]
fn a_parallel_aggregate_counts_no_expired_row() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = half_expired(dir.path(), 10_000);

    assert_eq!(
        aggregate(&collection, "SELECT COUNT(*) FROM a WHERE k = 1"),
        json!({"count": 5_000})
    );
}
