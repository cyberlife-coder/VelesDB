//! An `ORDER BY` that ranks every admitted row ranks all of them, however
//! many: the rows past the `MAX_LIMIT` ceiling are ranked too.
//!
//! The exhaustive fetch used to stop at `MAX_LIMIT` rows — in id order on a
//! graph, whose rows are listed by id — and rank only those, so the rows
//! past the ceiling, the highest ids, never took a place.

#![cfg(all(test, feature = "persistence"))]

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::collection::graph::{GraphEdge, GraphSchema};
use crate::collection::search::query::where_eval::GraphMatchEvalCache;
use crate::collection::search::query::{QuerySearchOptions, MAX_LIMIT};
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::velesql::Parser;

/// Rows past the `MAX_LIMIT` ceiling.
const PAST_CEILING: u64 = 5;

fn graph(embedding_dim: Option<usize>) -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let col = Collection::create_graph_collection(
        PathBuf::from(dir.path()),
        "rows",
        GraphSchema::schemaless(),
        embedding_dim,
        DistanceMetric::Cosine,
    )
    .expect("test: graph");
    (dir, col)
}

/// A graph of `MAX_LIMIT + PAST_CEILING` payload-only nodes, ids `1..`,
/// each `{"cat": "x", "price": id, "emb": [1, id]}`: the most similar `emb`
/// to `[1, 0]` and the cheapest node are the lowest ids, the least similar
/// and the dearest the highest.
fn past_the_ceiling() -> (tempfile::TempDir, Collection) {
    let (dir, col) = graph(None);
    let rows = u64::try_from(MAX_LIMIT).expect("test: ceiling") + PAST_CEILING;
    let ids: Vec<u64> = (1..=rows).collect();
    for chunk in ids.chunks(10_000) {
        #[allow(clippy::cast_precision_loss)]
        let payloads: Vec<(u64, Value)> = chunk
            .iter()
            .map(|&id| {
                (
                    id,
                    json!({"cat": "x", "price": id, "emb": [1.0, id as f32]}),
                )
            })
            .collect();
        let entries: Vec<(u64, &Value)> = payloads.iter().map(|(id, p)| (*id, p)).collect();
        col.store_node_payloads(&entries).expect("test: nodes");
    }
    (dir, col)
}

fn ids(col: &Collection, sql: &str) -> Vec<u64> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("test: parse {sql}: {e}"));
    let params: HashMap<String, Value> = HashMap::from([("q".to_string(), json!([1.0, 0.0]))]);
    col.execute_query(&query, &params)
        .unwrap_or_else(|e| panic!("test: {sql}: {e}"))
        .iter()
        .map(|r| r.point.id)
        .collect()
}

fn two_ranked_nodes() -> (tempfile::TempDir, Collection) {
    let (dir, col) = graph(Some(2));
    col.upsert([
        crate::point::Point::new(1, vec![1.0, 0.0], None),
        crate::point::Point::new(2, vec![0.0, 1.0], None),
    ])
    .expect("test: seed two ranked nodes");
    (dir, col)
}

#[test]
fn ordered_near_only_calibrates_the_index_on_the_ranked_search_path() {
    let (_dir, col) = two_ranked_nodes();
    let planner = &col.query.query_planner;
    let near = "SELECT * FROM rows WHERE vector NEAR $q";

    assert_eq!(ids(&col, &format!("{near} ORDER BY id DESC LIMIT 1")), [2]);
    assert_eq!(planner.stats().vector_query_count(), 1);
    assert_eq!(
        planner.cbo_sample_count(),
        0,
        "the scan never used the index"
    );

    let ranked = format!("{near} ORDER BY similarity() DESC LIMIT 1");
    assert_eq!(ids(&col, &ranked), [1]);
    assert_eq!(planner.stats().vector_query_count(), 2);
    assert_eq!(planner.cbo_sample_count(), 1);
}

#[test]
fn ordered_dense_search_enforces_the_effective_quality_at_the_capacity_boundary() {
    let (_dir, col) = two_ranked_nodes();
    let params = HashMap::from([("q".to_owned(), json!([1.0, 0.0]))]);
    for (predicate, order, first) in [
        ("vector NEAR $q", "ASC", 2),
        ("vector NEAR $q", "DESC", 1),
        ("similarity(vector, $q) >= 0.0", "ASC", 2),
        ("similarity(vector, $q) >= 0.0", "DESC", 1),
    ] {
        let sql =
            format!("SELECT * FROM rows WHERE {predicate} ORDER BY similarity() {order} LIMIT 1");
        let perfect = format!("{sql} WITH (mode = 'perfect')");
        for cap in [1, 2] {
            col.set_runtime_limits(crate::collection::RuntimeLimits {
                max_perfect_mode_vectors: cap,
                ..crate::collection::RuntimeLimits::default()
            });
            let result = col.execute_query_str(&perfect, &params);
            if cap == 2 {
                assert_eq!(result.expect("at capacity")[0].point.id, first);
            } else {
                assert!(
                    matches!(
                        result,
                        Err(crate::Error::GuardRail {
                            kind: crate::error::GuardRailKind::Capacity,
                            ..
                        })
                    ),
                    "{perfect}: {result:?}"
                );
                let overridden = format!("{sql} WITH (mode = 'perfect', ef_search = 64)");
                assert_eq!(ids(&col, &overridden), [first]);
            }
        }
    }
}

/// Plan the real ordered scan with a one-candidate exact-similarity ceiling.
fn planned_ids(col: &Collection, predicate: &str) -> crate::error::Result<Vec<u64>> {
    let sql = format!("SELECT * FROM rows AS r WHERE {predicate} ORDER BY price LIMIT 1");
    let query = Parser::parse(&sql).expect("test: query");
    let params = HashMap::from([("q".to_owned(), json!([1.0, 0.0]))]);
    let mut extracted = Collection::extract_query_components(&query.select, &params)?;
    assert!(matches!(
        col.select_fetch_window(
            &query.select,
            &mut extracted,
            &params,
            1,
            &mut QuerySearchOptions::default()
        ),
        super::super::fetch_window::FetchWindow::OrderedScan { pq_scores: false }
    ));
    let plan = col.ordered_scan_plan::<1>(
        &query.select,
        &params,
        &extracted,
        &mut GraphMatchEvalCache::default(),
        1,
    )?;
    Ok(plan.ids)
}

#[test]
fn ordered_scan_keeps_the_exact_similarity_candidate_ceiling() {
    let (_dir, col) = graph(None);
    let payload = json!({"price": 1});
    let exact = "similarity(vector, $q) < 0.5";
    col.store_node_payload(1, &payload).expect("test: node");
    assert_eq!(planned_ids(&col, exact).expect("at the ceiling"), [1]);
    col.store_node_payload(2, &payload).expect("test: node");
    let err = planned_ids(&col, exact).expect_err("above the exact scan ceiling");
    assert!(matches!(err, crate::error::Error::Config(msg) if msg.contains("scan limit of 1")));
    let edge = GraphEdge::new(1, 1, 2, "CITES").expect("test: edge");
    col.add_edge(edge).expect("test: add edge");
    let anchored = format!("{exact} AND MATCH (r)-[:CITES]->(other)");
    assert_eq!(planned_ids(&col, &anchored).expect("one candidate"), [1]);
    for predicate in ["price = 1", "similarity(vector, $q) > 0.5"] {
        assert_eq!(planned_ids(&col, predicate).expect("ordinary scan"), [1, 2]);
    }
}

#[test]
fn an_ordered_near_traverses_edges_to_payload_only_nodes() {
    let (_dir, col) = graph(Some(2));
    let embedded =
        [1, 2].map(|id| crate::point::Point::new(id, vec![1.0, 0.0], Some(json!({"price": id}))));
    col.upsert(embedded).expect("test: embedded nodes");
    col.store_node_payload(3, &json!({"price": 3}))
        .expect("test: payload-only node");
    let edge = GraphEdge::new(1, 1, 3, "CITES").expect("test: edge");
    col.add_edge(edge).expect("test: add edge");

    let all = "SELECT * FROM rows AS r WHERE";
    let edge = "MATCH (r)-[:CITES]->(other)";
    let near = "vector NEAR $q";
    assert_eq!(ids(&col, &format!("{all} {edge} ORDER BY price")), [1]);
    assert_eq!(ids(&col, &format!("{all} {near} ORDER BY price")), [1, 2]);
    let not_match = format!("{all} {near} AND NOT {edge} ORDER BY similarity() ASC");
    let disjunction = format!("{all} {near} AND (price = 99 OR {edge}) ORDER BY price");
    assert_eq!(
        (ids(&col, &not_match), ids(&col, &disjunction)),
        (vec![2], vec![1])
    );
}

#[test]
fn an_order_ranks_every_admitted_row_past_the_ceiling() {
    let (_dir, col) = past_the_ceiling();
    let top = u64::try_from(MAX_LIMIT).expect("test: ceiling") + PAST_CEILING;
    let filtered = "SELECT * FROM rows WHERE cat = 'x'";
    let all = "SELECT * FROM rows";
    let cases = [
        (
            format!("{filtered} ORDER BY price DESC LIMIT 3"),
            vec![top, top - 1, top - 2],
        ),
        (
            format!("{filtered} ORDER BY price DESC LIMIT 2 OFFSET 1"),
            vec![top - 1, top - 2],
        ),
        (
            format!("{filtered} ORDER BY similarity(emb, $q) ASC LIMIT 3"),
            vec![top, top - 1, top - 2],
        ),
        (format!("{filtered} ORDER BY price ASC LIMIT 2"), vec![1, 2]),
        (format!("{all} ORDER BY price DESC LIMIT 1"), vec![top]),
    ];
    for (sql, want) in cases {
        assert_eq!(ids(&col, &sql), want, "{sql}");
    }
    let ceiling = top - PAST_CEILING;
    let crossing = format!("{all} ORDER BY price ASC LIMIT 3 OFFSET {}", ceiling - 1);
    let beyond = format!("{all} ORDER BY price DESC LIMIT 2 OFFSET {ceiling}");
    let deep_pages = || (ids(&col, &crossing), ids(&col, &beyond));
    let expected = (
        vec![ceiling, ceiling + 1, ceiling + 2],
        vec![PAST_CEILING, PAST_CEILING - 1],
    );
    assert_eq!(deep_pages(), expected, "unindexed");
    col.create_index("price").expect("test: backfill price");
    assert_eq!(
        col.ordered_ids_if_covered("price", true, 0..1),
        Some(vec![top])
    );
    assert_eq!(deep_pages(), expected, "covering index");
}

#[path = "ordered_scan_range_tests.rs"]
mod range_candidates;
