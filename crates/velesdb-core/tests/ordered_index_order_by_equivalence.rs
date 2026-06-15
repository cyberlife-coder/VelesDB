#![cfg(all(test, feature = "persistence"))]
//! EPIC-081 phase 2 — index-backed `ORDER BY <field> LIMIT k` equivalence.
//!
//! The strongest correctness gate: for a fixed dataset, the SAME query must
//! return an **identical** id-sequence whether it runs through the exhaustive
//! fetch+sort (no secondary index) or the ordered-index fast path
//! (`create_index(col)`). Covers ASC, DESC, OFFSET, ties (duplicate keys),
//! k>n, k==n, k=0, and a NOT-fully-covered case (some rows lack the field →
//! must fall back to the exhaustive path and stay correct).

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

/// Builds a fresh "docs" collection from `(id, year)` rows. The vector is
/// irrelevant to a scalar ORDER BY; storage/id order deliberately differs from
/// year order so a truncate-before-sort bug would be visible.
fn build(rows: &[(u64, i64)]) -> (VectorCollection, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let collection = VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("create collection");
    let points: Vec<Point> = rows
        .iter()
        .map(|&(id, year)| Point::new(id, vec![1.0, 0.0], Some(json!({ "year": year }))))
        .collect();
    collection.upsert(points).expect("upsert");
    (collection, dir)
}

fn ids(results: &[velesdb_core::point::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// Runs `sql` on a fresh collection without any index (exhaustive path) and on
/// a second fresh collection with `create_index("year")` (index path), then
/// returns `(exhaustive_ids, index_ids)` so the caller can assert equality.
fn run_both(rows: &[(u64, i64)], sql: &str) -> (Vec<u64>, Vec<u64>) {
    let (no_index, _d1) = build(rows);
    let exhaustive = ids(&no_index
        .execute_query_str(sql, &HashMap::new())
        .expect("exhaustive"));

    let (with_index, _d2) = build(rows);
    with_index.create_index("year").expect("create_index");
    assert!(with_index.has_secondary_index("year"));
    let indexed = ids(&with_index
        .execute_query_str(sql, &HashMap::new())
        .expect("index"));

    (exhaustive, indexed)
}

/// Full dataset: ids 1..=8, with duplicate years to exercise tie-breaking.
/// Years: id1=2020, id2=2022, id3=2021, id4=2023, id5=2019, id6=2022,
/// id7=2020, id8=2019. So 2019→{5,8}, 2020→{1,7}, 2022→{2,6} are ties.
const ROWS: &[(u64, i64)] = &[
    (1, 2020),
    (2, 2022),
    (3, 2021),
    (4, 2023),
    (5, 2019),
    (6, 2022),
    (7, 2020),
    (8, 2019),
];

#[test]
fn desc_limit_matches_exhaustive_and_is_deterministic() {
    let (exhaustive, indexed) = run_both(ROWS, "SELECT * FROM docs ORDER BY year DESC LIMIT 5");
    assert_eq!(exhaustive, indexed);
    // DESC by year; ties broken by ascending id: 2023→4, 2022→{2,6}, 2021→3, 2020→1.
    assert_eq!(indexed, vec![4, 2, 6, 3, 1]);
}

#[test]
fn asc_limit_matches_exhaustive_and_is_deterministic() {
    let (exhaustive, indexed) = run_both(ROWS, "SELECT * FROM docs ORDER BY year ASC LIMIT 5");
    assert_eq!(exhaustive, indexed);
    // ASC by year; ties broken by ascending id: 2019→{5,8}, 2020→{1,7}, 2021→3.
    assert_eq!(indexed, vec![5, 8, 1, 7, 3]);
}

#[test]
fn desc_with_offset_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(
        ROWS,
        "SELECT * FROM docs ORDER BY year DESC LIMIT 3 OFFSET 2",
    );
    assert_eq!(exhaustive, indexed);
    // Full DESC order is [4,2,6,3,1,7,5,8]; skip 2, take 3 → [6,3,1].
    assert_eq!(indexed, vec![6, 3, 1]);
}

#[test]
fn asc_with_offset_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(
        ROWS,
        "SELECT * FROM docs ORDER BY year ASC LIMIT 3 OFFSET 3",
    );
    assert_eq!(exhaustive, indexed);
    // Full ASC order is [5,8,1,7,3,2,6,4]; skip 3, take 3 → [7,3,2].
    assert_eq!(indexed, vec![7, 3, 2]);
}

#[test]
fn ties_full_scan_matches_exhaustive() {
    // No LIMIT clause → engine default LIMIT 10 ≥ n, so the whole sorted set
    // is returned; both paths must agree on tie ordering across every bucket.
    let (exhaustive, indexed) = run_both(ROWS, "SELECT * FROM docs ORDER BY year DESC");
    assert_eq!(exhaustive, indexed);
    assert_eq!(indexed, vec![4, 2, 6, 3, 1, 7, 5, 8]);
}

#[test]
fn k_greater_than_n_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(ROWS, "SELECT * FROM docs ORDER BY year ASC LIMIT 100");
    assert_eq!(exhaustive, indexed);
    assert_eq!(indexed.len(), ROWS.len());
}

#[test]
fn k_equals_n_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(ROWS, "SELECT * FROM docs ORDER BY year DESC LIMIT 8");
    assert_eq!(exhaustive, indexed);
    assert_eq!(indexed.len(), ROWS.len());
}

#[test]
fn k_zero_matches_exhaustive() {
    let (exhaustive, indexed) = run_both(ROWS, "SELECT * FROM docs ORDER BY year DESC LIMIT 0");
    assert_eq!(exhaustive, indexed);
    assert!(indexed.is_empty());
}

#[test]
fn not_fully_covered_falls_back_and_stays_correct() {
    // id 9 lacks the `year` field, so the secondary index omits it but a full
    // ORDER BY places it FIRST for ASC (None < any value). The index path must
    // detect incomplete coverage and fall back to the exhaustive sort.
    let rows: &[(u64, i64)] = &[(1, 2020), (2, 2022), (3, 2021)];
    let (no_index, _d1) = build(rows);
    no_index
        .upsert(vec![Point::new(
            9,
            vec![1.0, 0.0],
            Some(json!({ "other": 1 })),
        )])
        .expect("upsert missing-field row");

    let (with_index, _d2) = build(rows);
    with_index
        .upsert(vec![Point::new(
            9,
            vec![1.0, 0.0],
            Some(json!({ "other": 1 })),
        )])
        .expect("upsert missing-field row");
    with_index.create_index("year").expect("create_index");

    let sql = "SELECT * FROM docs ORDER BY year ASC LIMIT 4";
    let exhaustive = ids(&no_index
        .execute_query_str(sql, &HashMap::new())
        .expect("exhaustive"));
    let indexed = ids(&with_index
        .execute_query_str(sql, &HashMap::new())
        .expect("index"));

    assert_eq!(exhaustive, indexed);
    // Row 9 (no year → sorts first ASC), then 2020→1, 2021→3, 2022→2.
    assert_eq!(indexed, vec![9, 1, 3, 2]);
}

#[test]
fn covered_after_backfill_then_uncovered_after_insert() {
    // Create the index on a fully-covered field, then insert a row missing the
    // field: coverage drops, so the next query must fall back, not silently
    // drop the new row.
    let rows: &[(u64, i64)] = &[(1, 2020), (2, 2022)];
    let (collection, _dir) = build(rows);
    collection.create_index("year").expect("create_index");

    // Fully covered: index path serves it (and must equal the exhaustive sort).
    let covered = ids(&collection
        .execute_query_str(
            "SELECT * FROM docs ORDER BY year ASC LIMIT 5",
            &HashMap::new(),
        )
        .expect("covered"));
    assert_eq!(covered, vec![1, 2]);

    // Insert a row without `year` → coverage breaks → fall back to exhaustive.
    collection
        .upsert(vec![Point::new(
            3,
            vec![1.0, 0.0],
            Some(json!({ "tag": "x" })),
        )])
        .expect("upsert");
    let after = ids(&collection
        .execute_query_str(
            "SELECT * FROM docs ORDER BY year ASC LIMIT 5",
            &HashMap::new(),
        )
        .expect("after"));
    // Row 3 has no year → sorts first ASC; the new row is NOT dropped.
    assert_eq!(after, vec![3, 1, 2]);
}

#[test]
fn high_id_above_u32_max_matches_exhaustive() {
    // ordered_ids returns Vec<u64> (no RoaringBitmap), so ids > u32::MAX are
    // fine on this path — no id-range restriction needed.
    let big = u64::from(u32::MAX) + 10;
    let rows: &[(u64, i64)] = &[(big, 2020), (2, 2022), (3, 2021)];
    let (no_index, _d1) = build(rows);
    let exhaustive = ids(&no_index
        .execute_query_str(
            "SELECT * FROM docs ORDER BY year DESC LIMIT 3",
            &HashMap::new(),
        )
        .expect("exhaustive"));

    let (with_index, _d2) = build(rows);
    with_index.create_index("year").expect("create_index");
    let indexed = ids(&with_index
        .execute_query_str(
            "SELECT * FROM docs ORDER BY year DESC LIMIT 3",
            &HashMap::new(),
        )
        .expect("index"));

    assert_eq!(exhaustive, indexed);
    // DESC: 2022→2, 2021→3, 2020→big.
    assert_eq!(indexed, vec![2, 3, big]);
}

#[test]
fn where_clause_keeps_exhaustive_path_and_correct() {
    // A WHERE clause disqualifies the index fast path; the result must still be
    // correct (and identical with or without the index present).
    let (no_index, _d1) = build(ROWS);
    let exhaustive = ids(&no_index
        .execute_query_str(
            "SELECT * FROM docs WHERE year >= 2021 ORDER BY year DESC LIMIT 2",
            &HashMap::new(),
        )
        .expect("exhaustive"));

    let (with_index, _d2) = build(ROWS);
    with_index.create_index("year").expect("create_index");
    let indexed = ids(&with_index
        .execute_query_str(
            "SELECT * FROM docs WHERE year >= 2021 ORDER BY year DESC LIMIT 2",
            &HashMap::new(),
        )
        .expect("index"));

    assert_eq!(exhaustive, indexed);
    // year >= 2021 → {2023→4, 2022→{2,6}, 2021→3}; DESC LIMIT 2 → [4, 2].
    assert_eq!(indexed, vec![4, 2]);
}

/// The index path must reproduce the exhaustive path's `.score`, not just the
/// id order — a plain `ORDER BY` scan scores every row 1.0 on both paths.
/// (Regression guard: the index route originally emitted 0.0, making `.score`
/// index-dependent for the exact queries it serves.)
#[test]
fn score_matches_exhaustive_not_just_ids() {
    let sql = "SELECT * FROM docs ORDER BY year DESC LIMIT 5";
    let pairs = |rs: &[velesdb_core::point::SearchResult]| -> Vec<(u64, f32)> {
        rs.iter().map(|r| (r.point.id, r.score)).collect()
    };

    let (no_index, _d1) = build(ROWS);
    let exhaustive = no_index
        .execute_query_str(sql, &HashMap::new())
        .expect("exhaustive");

    let (with_index, _d2) = build(ROWS);
    with_index.create_index("year").expect("create_index");
    let indexed = with_index
        .execute_query_str(sql, &HashMap::new())
        .expect("index");

    assert_eq!(
        pairs(&exhaustive),
        pairs(&indexed),
        "(id, score) must match"
    );
    assert!(
        indexed.iter().all(|r| (r.score - 1.0).abs() < f32::EPSILON),
        "plain ORDER BY scan scores 1.0 on the index path"
    );
}
