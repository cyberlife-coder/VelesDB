#![cfg(all(test, feature = "persistence"))]
//! Coverage: ordered-index `ORDER BY [WHERE] LIMIT k` decline branches.
//!
//! The equivalence suite (`ordered_index_order_by_equivalence`,
//! `ordered_index_multikey`) pins the *common* covered/uncovered cases. These
//! tests target the remaining decline branches of
//! `collection/search/query/ordered_index_scan.rs`:
//!
//! * the filtered route declining when the WHERE is **too selective** for the
//!   ordered-index walk to beat the exhaustive bitmap prefilter, *without*
//!   observing the advisor (the covering index is present, so it is not a gap);
//! * the filtered page short-circuiting on `LIMIT 0`;
//! * the route declining a WHERE that yields no usable metadata filter
//!   (`CONTAINS_TEXT` on a sort field is a metadata predicate; a `similarity()`
//!   WHERE is a non-metadata fetch).
//!
//! Every case asserts the indexed path returns the **same** rows as the
//! exhaustive path, so a decline is proven correct, and where possible uses the
//! ORDER BY index advisor as an observable signal of which branch fired.

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

/// Rows with a unique `year` per id (high cardinality) so an equality filter on
/// `year` is highly selective, plus a `tag` text field for `CONTAINS_TEXT`.
fn distinct_year_rows() -> Vec<Point> {
    (0..20u32)
        .map(|i| {
            Point::new(
                u64::from(i) + 1,
                vec![1.0, 0.0],
                Some(json!({ "year": 2000 + i64::from(i), "tag": format!("doc-{i}") })),
            )
        })
        .collect()
}

/// Runs `sql` on a fresh no-index collection (exhaustive) and on a fresh
/// indexed collection (`create_index(field)`), returning `(exhaustive, indexed)`
/// id sequences plus the indexed collection so the caller can inspect advice.
fn run_both(
    points: &[Point],
    field: &str,
    sql: &str,
) -> (Vec<u64>, Vec<u64>, VectorCollection, TempDir) {
    let da = TempDir::new().expect("dir");
    let a = mk(&da);
    a.upsert(points.to_vec()).expect("upsert exhaustive");
    let exhaustive = ids(&a
        .execute_query_str(sql, &HashMap::new())
        .expect("exhaustive query"));

    let db = TempDir::new().expect("dir");
    let b = mk(&db);
    b.upsert(points.to_vec()).expect("upsert indexed");
    b.create_index(field).expect("create_index");
    assert!(b.has_secondary_index(field), "index must exist");
    let indexed = ids(&b
        .execute_query_str(sql, &HashMap::new())
        .expect("indexed query"));

    (exhaustive, indexed, b, db)
}

/// A highly-selective equality WHERE on a covered field declines the filtered
/// route via the selectivity gate (`MIN_FILTERED_ROUTE_SELECTIVITY`). The route
/// must NOT observe the advisor (the covering index is present, so a covering
/// index is not the missing piece) yet still return the exhaustive result.
#[test]
fn highly_selective_equality_declines_without_observing() {
    let sql = "SELECT * FROM docs WHERE year = 2005 ORDER BY year DESC LIMIT 5";
    let (exhaustive, indexed, indexed_col, _d) = run_both(&distinct_year_rows(), "year", sql);

    assert_eq!(exhaustive, indexed, "selectivity decline must stay correct");
    // year = 2005 → id 6 only.
    assert_eq!(indexed, vec![6]);
    // The covering index IS present; the route declined purely on selectivity,
    // so it must not record the field as wanting an index.
    assert!(
        indexed_col.order_by_index_advice(1).is_empty(),
        "selectivity decline must not observe the advisor (index already covers)"
    );
}

/// A `CONTAINS_TEXT` predicate is a metadata filter (selectivity heuristic
/// 0.05 < 0.1), so the filtered route also declines on selectivity. It must
/// stay correct and, again, not observe the advisor.
#[test]
fn contains_text_filter_declines_without_observing() {
    let sql = "SELECT * FROM docs WHERE tag CONTAINS_TEXT 'doc-3' ORDER BY year ASC LIMIT 5";
    let (exhaustive, indexed, indexed_col, _d) = run_both(&distinct_year_rows(), "year", sql);

    assert_eq!(
        exhaustive, indexed,
        "CONTAINS_TEXT decline must stay correct"
    );
    // tag = "doc-3" (substring "doc-3") → id 4 only (year 2003).
    assert_eq!(indexed, vec![4]);
    assert!(
        indexed_col.order_by_index_advice(1).is_empty(),
        "selectivity decline must not observe the advisor"
    );
}

/// `LIMIT 0` under a broad (route-eligible) WHERE drives the filtered page's
/// `limit == 0` short-circuit. Both paths must return zero rows.
#[test]
fn filtered_route_limit_zero_returns_empty() {
    let sql = "SELECT * FROM docs WHERE year >= 2000 ORDER BY year DESC LIMIT 0";
    let (exhaustive, indexed, _col, _d) = run_both(&distinct_year_rows(), "year", sql);

    assert!(exhaustive.is_empty(), "exhaustive LIMIT 0 is empty");
    assert_eq!(exhaustive, indexed, "LIMIT 0 must match on both paths");
}

/// A broad WHERE (`>= 2000`, selectivity 1.0) with LIMIT > 0 *does* take the
/// filtered route through `collect_filtered_page`; the indexed result must equal
/// the exhaustive filter→sort→limit. Sibling to the LIMIT 0 case so the page
/// builder is exercised both empty and non-empty.
#[test]
fn filtered_route_broad_where_matches_exhaustive() {
    let sql = "SELECT * FROM docs WHERE year >= 2000 ORDER BY year DESC LIMIT 3";
    let (exhaustive, indexed, _col, _d) = run_both(&distinct_year_rows(), "year", sql);

    assert_eq!(
        exhaustive, indexed,
        "broad-filter route must match exhaustive"
    );
    // DESC: 2019→20, 2018→19, 2017→18.
    assert_eq!(indexed, vec![20, 19, 18]);
}

/// A vector-similarity WHERE is a non-metadata fetch, so the ordered-index route
/// declines outright (it never classifies as a metadata filter). The ORDER BY
/// still applies on the exhaustive path; both paths must agree on the rows.
#[test]
fn similarity_where_is_not_routed_and_matches_exhaustive() {
    let sql = "SELECT * FROM docs WHERE similarity([1.0, 0.0]) > 0.0 ORDER BY year DESC LIMIT 3";
    let da = TempDir::new().expect("dir");
    let a = mk(&da);
    a.upsert(distinct_year_rows()).expect("upsert exhaustive");
    let exhaustive = a.execute_query_str(sql, &HashMap::new());

    let db = TempDir::new().expect("dir");
    let b = mk(&db);
    b.upsert(distinct_year_rows()).expect("upsert indexed");
    b.create_index("year").expect("create_index");
    let indexed = b.execute_query_str(sql, &HashMap::new());

    match (exhaustive, indexed) {
        (Ok(ex), Ok(idx)) => {
            // The non-metadata fetch declines the ordered-index route on the
            // indexed collection; both collections run the same (ranked) path,
            // so the returned id sets must be identical.
            let mut ex_ids = ids(&ex);
            let mut idx_ids = ids(&idx);
            ex_ids.sort_unstable();
            idx_ids.sort_unstable();
            assert_eq!(ex_ids, idx_ids, "similarity WHERE must match across paths");
        }
        (Err(_), Err(_)) => {
            // Both paths reject the shape identically — also acceptable: the
            // point is that the index does not change behaviour.
        }
        (ex, idx) => panic!("index changed similarity-WHERE behaviour: {ex:?} vs {idx:?}"),
    }
}
