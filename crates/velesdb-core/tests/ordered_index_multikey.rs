#![cfg(all(test, feature = "persistence"))]
//! EPIC-081 phase 3c — multi-column `ORDER BY` equivalence.
//!
//! `ORDER BY <lead_field>, <more…>` on a covering lead-field index walks whole
//! leading lead-key buckets (in lead order) until ≥ k rows, then applies the
//! exhaustive multi-key sort. For a fixed dataset the SAME query must return an
//! identical id-sequence with or without the index. Covers ASC/DESC lead,
//! secondary direction, OFFSET, ties on both keys, heterogeneous lead types
//! (Bool/Number/String — the index `JsonValue` Ord must match
//! `compare_json_values`), k>n, the uncovered-lead fall-back, the
//! multi-key+WHERE decline, and TTL-expired rows.

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

fn mk(dir: &TempDir) -> VectorCollection {
    VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("create collection")
}

fn ids(rs: &[velesdb_core::point::SearchResult]) -> Vec<u64> {
    rs.iter().map(|r| r.point.id).collect()
}

/// Runs `sql` on a fresh collection without any index (exhaustive) and on a
/// second with `create_index(lead)` (the multi-key index path); returns
/// `(exhaustive_ids, index_ids)`.
fn run_both(points: &[Point], lead: &str, sql: &str) -> (Vec<u64>, Vec<u64>) {
    let da = TempDir::new().expect("dir");
    let a = mk(&da);
    a.upsert(points.to_vec()).expect("upsert");
    let exhaustive = ids(&a
        .execute_query_str(sql, &HashMap::new())
        .expect("exhaustive"));

    let db = TempDir::new().expect("dir");
    let b = mk(&db);
    b.upsert(points.to_vec()).expect("upsert");
    b.create_index(lead).expect("create_index");
    let indexed = ids(&b.execute_query_str(sql, &HashMap::new()).expect("index"));

    (exhaustive, indexed)
}

/// cat ∈ {a, b}; (a,2021) is a two-key tie across ids 3 & 5.
fn cat_year_rows() -> Vec<Point> {
    vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({ "cat": "a", "year": 2020 }))),
        Point::new(2, vec![1.0, 0.0], Some(json!({ "cat": "b", "year": 2022 }))),
        Point::new(3, vec![1.0, 0.0], Some(json!({ "cat": "a", "year": 2021 }))),
        Point::new(4, vec![1.0, 0.0], Some(json!({ "cat": "b", "year": 2021 }))),
        Point::new(5, vec![1.0, 0.0], Some(json!({ "cat": "a", "year": 2021 }))),
    ]
}

#[test]
fn lead_asc_secondary_desc_full_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(
        &cat_year_rows(),
        "cat",
        "SELECT * FROM docs ORDER BY cat ASC, year DESC LIMIT 5",
    );
    assert_eq!(exhaustive, indexed);
    // cat a: year desc 2021(3),2021(5),2020(1); cat b: 2022(2),2021(4) → [3,5,1,2,4].
    assert_eq!(indexed, vec![3, 5, 1, 2, 4]);
}

#[test]
fn lead_asc_prunes_trailing_bucket_for_topk() {
    let (exhaustive, indexed) = run_both(
        &cat_year_rows(),
        "cat",
        "SELECT * FROM docs ORDER BY cat ASC, year DESC LIMIT 2",
    );
    assert_eq!(exhaustive, indexed);
    // LIMIT 2 needs only bucket `a` (3 rows): [3, 5]. Bucket `b` is pruned.
    assert_eq!(indexed, vec![3, 5]);
}

#[test]
fn lead_desc_secondary_asc_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(
        &cat_year_rows(),
        "cat",
        "SELECT * FROM docs ORDER BY cat DESC, year ASC LIMIT 5",
    );
    assert_eq!(exhaustive, indexed);
    // cat b: year asc 2021(4),2022(2); cat a: 2020(1),2021(3),2021(5) → [4,2,1,3,5].
    assert_eq!(indexed, vec![4, 2, 1, 3, 5]);
}

#[test]
fn offset_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(
        &cat_year_rows(),
        "cat",
        "SELECT * FROM docs ORDER BY cat ASC, year DESC LIMIT 2 OFFSET 1",
    );
    assert_eq!(exhaustive, indexed);
    // Full [3,5,1,2,4]; skip 1, take 2 → [5, 1].
    assert_eq!(indexed, vec![5, 1]);
}

#[test]
fn heterogeneous_lead_types_match_exhaustive() {
    // Mixed lead-key types: the index JsonValue Ord (Bool < Number < String)
    // must agree with compare_json_values for the prefix to be the right buckets.
    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({ "cat": true, "year": 2020 })),
        ),
        Point::new(2, vec![1.0, 0.0], Some(json!({ "cat": 5, "year": 2021 }))),
        Point::new(3, vec![1.0, 0.0], Some(json!({ "cat": "x", "year": 2022 }))),
        Point::new(
            4,
            vec![1.0, 0.0],
            Some(json!({ "cat": true, "year": 2019 })),
        ),
        Point::new(5, vec![1.0, 0.0], Some(json!({ "cat": 5, "year": 2023 }))),
    ];
    let (exhaustive, indexed) = run_both(
        &points,
        "cat",
        "SELECT * FROM docs ORDER BY cat ASC, year DESC LIMIT 5",
    );
    assert_eq!(exhaustive, indexed);
    // Bool true: 2020(1),2019(4); Number 5: 2023(5),2021(2); String "x": 2022(3).
    assert_eq!(indexed, vec![1, 4, 5, 2, 3]);
}

#[test]
fn k_greater_than_n_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(
        &cat_year_rows(),
        "cat",
        "SELECT * FROM docs ORDER BY cat ASC, year DESC LIMIT 100",
    );
    assert_eq!(exhaustive, indexed);
    assert_eq!(indexed.len(), 5);
}

#[test]
fn uncovered_lead_falls_back_and_matches_exhaustive() {
    // id 9 lacks the lead field `cat` → coverage breaks → the multi-key route
    // declines and the exhaustive sort runs (placing the missing-cat row first
    // for ASC). Both paths must agree.
    let mut points = cat_year_rows();
    points.push(Point::new(9, vec![1.0, 0.0], Some(json!({ "year": 2099 }))));
    let (exhaustive, indexed) = run_both(
        &points,
        "cat",
        "SELECT * FROM docs ORDER BY cat ASC, year DESC LIMIT 6",
    );
    assert_eq!(exhaustive, indexed);
    // Missing cat sorts first (None < any): 9, then a-group [3,5,1], then b [2,4].
    assert_eq!(indexed, vec![9, 3, 5, 1, 2, 4]);
}

#[test]
fn multikey_with_where_declines_but_stays_correct() {
    // Multi-key + WHERE is not routed (decline → exhaustive); result still correct
    // and identical with or without the index.
    let (exhaustive, indexed) = run_both(
        &cat_year_rows(),
        "cat",
        "SELECT * FROM docs WHERE year >= 2021 ORDER BY cat ASC, year DESC LIMIT 5",
    );
    assert_eq!(exhaustive, indexed);
    // year>=2021 → a:{2021(3),2021(5)}, b:{2022(2),2021(4)}; cat asc, year desc → [3,5,2,4].
    assert_eq!(indexed, vec![3, 5, 2, 4]);
}

#[test]
fn expired_row_in_prefix_falls_back_and_matches_exhaustive() {
    // A TTL-expired row in the lead-key prefix is dropped by get(); the route
    // declines (the page could need a trailing-bucket row) so the exhaustive
    // path runs. id 3 (cat a, 2021) is expired.
    let points = vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({ "cat": "a", "year": 2020 }))),
        Point::new(2, vec![1.0, 0.0], Some(json!({ "cat": "b", "year": 2022 }))),
        Point::new(
            3,
            vec![1.0, 0.0],
            Some(json!({ "cat": "a", "year": 2021, "_veles_expires_at": 1 })),
        ),
        Point::new(4, vec![1.0, 0.0], Some(json!({ "cat": "b", "year": 2021 }))),
        Point::new(5, vec![1.0, 0.0], Some(json!({ "cat": "a", "year": 2019 }))),
    ];
    let (exhaustive, indexed) = run_both(
        &points,
        "cat",
        "SELECT * FROM docs ORDER BY cat ASC, year DESC LIMIT 4",
    );
    assert_eq!(exhaustive, indexed);
    // id3 expired & dropped; live cat a: 2020(1),2019(5); cat b: 2022(2),2021(4) → [1,5,2,4].
    assert_eq!(indexed, vec![1, 5, 2, 4]);
}
