//! Core-level regression tests for MATCH `RETURN ... ORDER BY` + post-sort
//! `LIMIT`, pinning two contracts that surfaces beyond the SQL pipeline depend
//! on:
//!
//! 1. The SQL `/query` path must return the GLOBAL top-K under an ORDER BY, not
//!    the sorted-top-K of the first-K nodes traversed (backlog #1b). Before the
//!    fix, `execute_match_with_context` early-broke traversal at
//!    `return_clause.limit` BEFORE the sort, so the LIMIT was applied to the
//!    first-K-traversed set instead of the globally ordered one.
//!
//! 2. The public ordered-MATCH entry point (`match_query_ordered`, the single
//!    method later non-SQL surfaces route through) must return the SAME ordered
//!    ids as the SQL `/query` path (backlog #1 core).
//!
//! Dataset (`cdocs`, 2-dim cosine, all `:Doc`): six nodes whose ids ascend with
//! their `year`, so the label-index traversal order (ascending node id) is the
//! OPPOSITE of the year-DESC order. Hence the global top-2 by year (ids 6, 5)
//! is disjoint from the first-2 traversed (ids 1, 2) — the divergence that
//! exposes the early-break bug.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, Point};

use super::helpers::create_test_db;

/// Number of `:Doc` nodes seeded; chosen so LIMIT 2 ≪ N and the global top-K
/// is provably disjoint from the first-K-traversed.
const DOC_COUNT: u64 = 6;

/// Builds `cdocs`: ids 1..=6 with `year = 2000 + id`, all labelled `:Doc`.
fn setup_core_docs(db: &Database) {
    db.create_vector_collection("cdocs", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create cdocs");
    let vc = db.get_vector_collection("cdocs").expect("test: get cdocs");
    let points: Vec<Point> = (1..=DOC_COUNT)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.0],
                Some(json!({"_labels": ["Doc"], "year": 2000 + id})),
            )
        })
        .collect();
    vc.upsert(points).expect("test: upsert cdocs");
}

/// Runs `sql` through the SQL `/query` pipeline against `cdocs`, returning ids.
fn sql_ids(db: &Database, sql: &str) -> Vec<u64> {
    let mut params = HashMap::new();
    params.insert("_collection".to_string(), json!("cdocs"));
    let query = Parser::parse(sql).expect("test: parse MATCH ORDER BY");
    db.execute_query(&query, &params)
        .expect("test: execute MATCH ORDER BY")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

const ORDER_BY_TOP2: &str = "MATCH (d:Doc) RETURN d ORDER BY d.year DESC LIMIT 2";

/// #1b: the SQL pipeline must return the GLOBAL year-DESC top-2 (ids 6, 5), not
/// the sorted first-2-traversed (ids 2, 1). The early-break-before-sort bug
/// returned `[2, 1]`.
#[test]
fn scenario_match_order_by_limit_returns_global_top_k() {
    let (_dir, db) = create_test_db();
    setup_core_docs(&db);
    let ids = sql_ids(&db, ORDER_BY_TOP2);
    assert_eq!(
        ids,
        vec![6u64, 5],
        "ORDER BY year DESC LIMIT 2 must be the GLOBAL top-2, not the \
         sorted-first-2-traversed"
    );
}

/// #1 core: the public `match_query_ordered` entry point must return the SAME
/// ordered top-K as the SQL `/query` path (single source of truth).
#[test]
fn scenario_match_query_ordered_matches_sql_path() {
    let (_dir, db) = create_test_db();
    setup_core_docs(&db);

    let sql = sql_ids(&db, ORDER_BY_TOP2);

    let params: HashMap<String, serde_json::Value> = HashMap::new();
    let match_clause = Parser::parse(ORDER_BY_TOP2)
        .expect("test: parse")
        .match_clause
        .expect("test: MATCH clause present");
    let vc = db.get_vector_collection("cdocs").expect("test: get cdocs");
    let ordered: Vec<u64> = vc
        .match_query_ordered(&match_clause, &params)
        .expect("test: match_query_ordered")
        .iter()
        .map(|r| r.node_id)
        .collect();

    assert_eq!(
        ordered, sql,
        "match_query_ordered must return the same ordered ids as the SQL path"
    );
    assert_eq!(ordered, vec![6u64, 5], "ordered top-2 by year DESC");
}

// --- A VectorFirst-selecting MATCH under a payload ORDER BY (backlog #1b) ---
//
// A start-similarity single-node MATCH would otherwise select the VectorFirst
// strategy, which fetches only an approximate-HNSW, similarity-ranked prefix —
// it cannot yield the global top-K under an ORDER BY on a non-similarity field.
// The planner now routes such queries to GraphFirst's EXACT label enumeration,
// making the result deterministic (no HNSW recall dependence).
//
// Dataset (`vdocs`, 2-dim cosine): vectors fan from 0deg (id 1, best cosine to
// the query [1,0]) to ~50deg (id 6, worst cosine but still cos>=0.64 > 0.5),
// while `year` ascends with id. So the global year-DESC top-2 (ids 6, 5 — the
// least-similar, highest-year) is disjoint from the similarity-top-2 (ids 1, 2).

/// Builds `vdocs`: ids 1..=6, `year = 2000 + id`, vectors fanning 0..~50deg.
fn setup_vdocs(db: &Database) {
    db.create_vector_collection("vdocs", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create vdocs");
    let vc = db.get_vector_collection("vdocs").expect("test: get vdocs");
    let points: Vec<Point> = (1..=DOC_COUNT)
        .map(|id| {
            #[allow(clippy::cast_precision_loss)]
            let angle = (id - 1) as f32 * (50.0_f32.to_radians() / (DOC_COUNT - 1) as f32);
            Point::new(
                id,
                vec![angle.cos(), angle.sin()],
                Some(json!({"_labels": ["Doc"], "year": 2000 + id})),
            )
        })
        .collect();
    vc.upsert(points).expect("test: upsert vdocs");
}

fn vdocs_ids(db: &Database, sql: &str) -> Vec<u64> {
    let mut params = HashMap::new();
    params.insert("_collection".to_string(), json!("vdocs"));
    params.insert("q".to_string(), json!([1.0, 0.0]));
    let query = Parser::parse(sql).expect("test: parse vdocs MATCH");
    db.execute_query(&query, &params)
        .expect("test: execute vdocs MATCH")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

/// #1b (VectorFirst-selecting query): a start-similarity MATCH with an ORDER BY
/// on the `year` payload must return the GLOBAL year-DESC top-2 (ids 6, 5), not
/// the year-sort of the similarity-ranked prefix (ids 2, 1).
#[test]
fn scenario_similarity_match_order_by_payload_returns_global_top_k() {
    let (_dir, db) = create_test_db();
    setup_vdocs(&db);
    let ids = vdocs_ids(
        &db,
        "MATCH (d:Doc) WHERE similarity(d, $q) > 0.5 RETURN d ORDER BY d.year DESC LIMIT 2",
    );
    assert_eq!(
        ids,
        vec![6u64, 5],
        "similarity MATCH + ORDER BY year DESC LIMIT 2 must be the GLOBAL top-2, \
         not the year-sort of the similarity-ranked prefix"
    );
}
