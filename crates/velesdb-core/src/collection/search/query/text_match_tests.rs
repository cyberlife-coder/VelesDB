//! A text `MATCH` in a SELECT WHERE: a conjunct under `AND` ranks the rows
//! by BM25, the rest of the WHERE their admission filter — exact, however
//! far below the best a matching row ranks. Any other `MATCH` is a test of
//! membership in its BM25 hits; one that would rank beside a ranking no
//! executor fuses with text is refused, never dropped.

use std::collections::HashMap;
use std::path::Path;

use serde_json::json;

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::{Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// Ids 1..=24 are category `a` with a short "banana" text, which ranks
/// first; ids 25..=30 are category `b` with a long one, ranking below all of
/// them; ids 31..=33 are category `b` without "banana" at all.
fn collection(dir: &Path) -> Collection {
    let collection =
        Collection::create(dir.join("v"), 2, DistanceMetric::Cosine).expect("test: collection");
    collection
        .upsert((1..=33).map(|id: u64| {
            let (content, cat) = match id {
                1..=24 => ("banana".to_string(), "a"),
                25..=30 => (format!("banana {}", "filler ".repeat(30)), "b"),
                _ => ("cherry".to_string(), "b"),
            };
            #[allow(clippy::cast_precision_loss)]
            let vector = vec![1.0, id as f32];
            Point::new(id, vector, Some(json!({"content": content, "cat": cat})))
        }))
        .expect("test: upsert");
    collection
}

fn query(collection: &Collection, sql: &str) -> crate::error::Result<Vec<SearchResult>> {
    collection.execute_query_str(sql, &HashMap::new())
}

#[test]
fn a_match_and_a_metadata_condition_take_both() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection(dir.path());

    for sql in [
        "SELECT * FROM v WHERE content MATCH 'banana' AND cat = 'b' LIMIT 2",
        "SELECT * FROM v WHERE cat = 'b' AND content MATCH 'banana' LIMIT 2",
        "SELECT * FROM v WHERE (content MATCH 'banana') AND (cat = 'b') LIMIT 2",
    ] {
        let hits = query(&collection, sql).expect("test: query");
        assert_eq!(hits.len(), 2, "{sql}: {:?}", ids(&hits));
        assert!(
            ids(&hits).iter().all(|id| (25..=30).contains(id)),
            "{sql}: {:?}",
            ids(&hits)
        );
    }
}

#[test]
fn a_match_and_a_metadata_condition_return_every_row_both_take_up_to_the_limit() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection(dir.path());

    let hits = query(
        &collection,
        "SELECT * FROM v WHERE content MATCH 'banana' AND cat = 'b' LIMIT 100",
    )
    .expect("test: query");

    let mut found = ids(&hits);
    found.sort_unstable();
    assert_eq!(found, [25, 26, 27, 28, 29, 30]);
}

#[test]
fn a_match_no_row_holds_returns_nothing_whatever_else_matches() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection(dir.path());

    let hits = query(
        &collection,
        "SELECT * FROM v WHERE content MATCH 'zzz' AND cat = 'b' LIMIT 10",
    )
    .expect("test: query");

    assert!(hits.is_empty(), "{:?}", ids(&hits));
}

#[test]
fn a_match_that_ranks_nothing_selects_its_bm25_hits() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection(dir.path());
    let sorted = |sql: &str| {
        let mut found = ids(&query(&collection, sql).expect(sql));
        found.sort_unstable();
        found
    };

    assert_eq!(
        sorted("SELECT * FROM v WHERE content MATCH 'cherry' OR cat = 'a' LIMIT 100"),
        (1..=24).chain(31..=33).collect::<Vec<u64>>()
    );
    assert_eq!(
        sorted("SELECT * FROM v WHERE NOT (content MATCH 'banana') LIMIT 100"),
        [31, 32, 33]
    );
    assert_eq!(
        sorted(
            "SELECT * FROM v WHERE cat = 'b' AND (content MATCH 'cherry' OR cat = 'a') LIMIT 100"
        ),
        [31, 32, 33]
    );
    assert_eq!(
        sorted(
            "SELECT * FROM v WHERE content MATCH 'banana' AND \
             similarity(vector, [1.0, 0.0]) < 0.2 LIMIT 100"
        ),
        (5..=30).collect::<Vec<u64>>()
    );
}

#[test]
fn a_second_ranking_match_is_refused() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection(dir.path());

    let sql = "SELECT * FROM v WHERE content MATCH 'banana' AND content MATCH 'cherry' LIMIT 10";
    let err = query(&collection, sql).expect_err(sql);

    assert_eq!(err.code(), "VELES-010", "{sql}: {err}");
    assert!(err.to_string().contains("MATCH"), "{sql}: {err}");
}
