//! Tests for the SELECT strategy dispatch (`dispatch_vector_with_strategy`)
//! and the cost-model-driven metadata fallback.

#![cfg(all(test, feature = "persistence"))]

use crate::collection::search::query::QuerySearchOptions;
use crate::point::{Point, SearchResult};
use crate::test_fixtures::fixtures::{make_point_with_payload, setup_collection};
use crate::velesql::ExecutionStrategy;

fn ids_of(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

// ============================================================================
// A. The two strategies are distinct plans
// ============================================================================

/// Shared fixture: 20 `category="tech"` points whose second vector component
/// increases with the id, so cosine similarity to `[1,0,0,0]` is strictly
/// monotone in the id (id 1 = closest). Distinct scores make every strategy's
/// output fully ordered and deterministic.
fn setup_parallel_fixture() -> (tempfile::TempDir, crate::collection::Collection) {
    let (dir, col) = setup_collection(4);
    let points: Vec<Point> = (1..=20u64)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = vec![1.0, i as f32 * 0.05, 0.0, 0.0];
            make_point_with_payload(i, v, serde_json::json!({"category": "tech"}))
        })
        .collect();
    col.upsert(points).expect("test: upsert");
    (dir, col)
}

fn tech_filter() -> crate::filter::Filter {
    crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "category".into(),
        value: serde_json::json!("tech"),
    })
}

/// GIVEN a filtered collection
/// WHEN dispatching with a FORCED `ExecutionStrategy::GraphFirst`
/// THEN the result equals the exhaustive `scan_and_score_by_vector`
///      realization (a full metadata scan scored by vector similarity),
///      physically distinct from the HNSW path.
#[test]
fn test_graph_first_strategy_matches_exhaustive_scan_reference() {
    let (_dir, col) = setup_parallel_fixture();
    let filter = tech_filter();
    let query = [1.0, 0.0, 0.0, 0.0];
    let opts = QuerySearchOptions::default();
    let (k, limit) = (10, 3);

    let graph_first = col
        .dispatch_vector_with_strategy(
            &query,
            &filter,
            ExecutionStrategy::GraphFirst,
            k,
            limit,
            &opts,
        )
        .expect("test: graph-first dispatch");

    let expected = col.scan_and_score_by_vector(&filter, &query, limit);

    assert_eq!(
        ids_of(&graph_first),
        ids_of(&expected),
        "GraphFirst must equal the exhaustive scan-and-score realization"
    );
    assert_eq!(graph_first.len(), limit, "GraphFirst returns top-`limit`");
    assert_eq!(graph_first[0].point.id, 1, "closest point ranks first");
}

/// GIVEN a filtered collection
/// WHEN dispatching with a FORCED `ExecutionStrategy::VectorFirst`
/// THEN the result equals the filtered-HNSW `search_with_filter_and_opts`
///      realization (up to `k` candidates), distinct from the GraphFirst scan.
#[test]
fn test_vector_first_strategy_matches_hnsw_filtered_reference() {
    let (_dir, col) = setup_parallel_fixture();
    let filter = tech_filter();
    let query = [1.0, 0.0, 0.0, 0.0];
    let opts = QuerySearchOptions::default();
    let (k, limit) = (10, 3);

    let vector_first = col
        .dispatch_vector_with_strategy(
            &query,
            &filter,
            ExecutionStrategy::VectorFirst,
            k,
            limit,
            &opts,
        )
        .expect("test: vector-first dispatch");

    let expected = col
        .search_with_filter_and_opts(&query, k, &filter, &opts)
        .expect("test: vector reference");

    assert_eq!(
        ids_of(&vector_first),
        ids_of(&expected),
        "VectorFirst must equal the filtered-HNSW realization"
    );
}

/// GIVEN the same data, filter, query, k, and limit
/// WHEN dispatched under GraphFirst and VectorFirst
/// THEN the two strategies are observably distinct physical plans: the
///      VectorFirst set (up to `k` HNSW candidates) differs from the
///      GraphFirst set (top-`limit` exhaustive), and every row VectorFirst
///      returns is also one GraphFirst's exhaustive scan admits — which is
///      why no plan runs both.
#[test]
fn test_two_strategies_yield_distinct_result_sets() {
    let (_dir, col) = setup_parallel_fixture();
    let filter = tech_filter();
    let query = [1.0, 0.0, 0.0, 0.0];
    let opts = QuerySearchOptions::default();
    let (k, limit) = (10, 3);

    let dispatch = |strategy, limit| {
        col.dispatch_vector_with_strategy(&query, &filter, strategy, k, limit, &opts)
            .expect("test: dispatch")
    };
    let graph_first = dispatch(ExecutionStrategy::GraphFirst, limit);
    let vector_first = dispatch(ExecutionStrategy::VectorFirst, limit);

    assert_ne!(
        ids_of(&graph_first),
        ids_of(&vector_first),
        "GraphFirst and VectorFirst must produce distinct result-sets"
    );
    let everything = ids_of(&dispatch(ExecutionStrategy::GraphFirst, 100));
    assert!(
        ids_of(&vector_first)
            .iter()
            .all(|id| everything.contains(id)),
        "GraphFirst's scan admits every row VectorFirst returns"
    );
}

// ============================================================================
// B. Cost-model-driven metadata fallback (audit F-4.7, issue #1391)
// ============================================================================

use crate::collection::stats::{CollectionStats, ColumnStats};
use crate::collection::Collection;
use crate::velesql::{CompareOp, Comparison, Condition, Value};

/// Builds a velesql `col = "value"` string-equality condition.
fn eq_string_cond(column: &str, value: &str) -> Condition {
    Condition::Comparison(Comparison {
        column: column.to_string(),
        operator: CompareOp::Eq,
        value: Value::String(value.to_string()),
    })
}

/// Hand-builds `CollectionStats` whose `estimate_selectivity(column)` resolves
/// to `1 / distinct_values` (no histogram → cardinality fallback).
fn stats_with_cardinality(total: u64, column: &str, distinct_values: u64) -> CollectionStats {
    let mut field_stats = std::collections::HashMap::new();
    field_stats.insert(
        column.to_string(),
        ColumnStats {
            name: column.to_string(),
            distinct_values,
            distinct_count: distinct_values,
            ..Default::default()
        },
    );
    CollectionStats {
        total_points: total,
        row_count: total,
        field_stats,
        ..Default::default()
    }
}

/// The fallback bascule is driven by the cost model: for an *identical*
/// candidate count and execution limit, a high-selectivity predicate (few
/// matches → cheap full scan with early exit) declines the candidate path,
/// while a low-selectivity predicate (many matches → expensive full scan)
/// prefers it. Only the stats differ between the two calls.
#[test]
fn test_candidate_scan_switch_is_cost_driven() {
    let cond = eq_string_cond("cat", "x");
    let (candidate_count, exec_limit, total) = (5_000, 10, 100_000);

    // distinct_values = 2 → selectivity 0.5 → a full scan finds `exec_limit`
    // matches after ~20 rows, far cheaper than hydrating 5_000 candidates.
    let high_sel = stats_with_cardinality(total, "cat", 2);
    assert!(
        !Collection::candidate_scan_preferred(&high_sel, candidate_count, exec_limit, Some(&cond)),
        "high selectivity → full scan is cheaper, candidate path declined"
    );

    // distinct_values = total → selectivity ~1/total → a full scan must visit
    // (nearly) the whole collection, so hydrating 5_000 candidates wins.
    let low_sel = stats_with_cardinality(total, "cat", total);
    assert!(
        Collection::candidate_scan_preferred(&low_sel, candidate_count, exec_limit, Some(&cond)),
        "low selectivity → candidate scan is cheaper"
    );
}

/// Guardrails and edge cases: the `.max(1000)` floor, empty candidate sets,
/// un-analysed stats, and a zero execution limit must all resolve safely
/// without panicking or dividing by zero.
#[test]
fn test_candidate_scan_floor_and_edge_cases() {
    let cond = eq_string_cond("cat", "x");
    // High-selectivity stats that would otherwise decline the candidate path.
    let high_sel = stats_with_cardinality(100_000, "cat", 2);

    // Floor: candidate sets ≤ 1000 always take the candidate path, regardless
    // of how cheap the full scan looks.
    assert!(Collection::candidate_scan_preferred(
        &high_sel,
        1_000,
        10,
        Some(&cond)
    ));
    // Empty candidate set is trivially below the floor.
    assert!(Collection::candidate_scan_preferred(
        &high_sel,
        0,
        10,
        Some(&cond)
    ));
    // Empty / un-analysed collection (total == 0) → candidate path, no panic.
    let empty = CollectionStats::default();
    assert!(Collection::candidate_scan_preferred(
        &empty,
        5_000,
        10,
        Some(&cond)
    ));
    // exec_limit == 0 must not divide by zero; low selectivity still prefers
    // the candidate path.
    let low_sel = stats_with_cardinality(100_000, "cat", 100_000);
    assert!(Collection::candidate_scan_preferred(
        &low_sel,
        5_000,
        0,
        Some(&cond)
    ));
}

/// When no condition tree is available (SELECT * / empty filter) the helper
/// falls back to the historical `exec_limit * 50 .max(1000)` budget so those
/// paths keep their previous behaviour unchanged.
#[test]
fn test_candidate_scan_none_cond_uses_legacy_budget() {
    let stats = stats_with_cardinality(100_000, "cat", 2);
    // exec_limit 10 → budget max(500, 1000) = 1000; 5_000 > 1000 → full scan.
    assert!(!Collection::candidate_scan_preferred(
        &stats, 5_000, 10, None
    ));
    // exec_limit 100 → budget 5_000; 1_500 ≤ 5_000 → candidate path.
    assert!(Collection::candidate_scan_preferred(
        &stats, 1_500, 100, None
    ));
}

/// Result equivalence: whichever physical path the cost model selects, a
/// metadata scan returns exactly the rows matching the predicate. Uses an
/// indexed field and a large limit (no truncation) so the full matching set is
/// compared against an independent brute-force reference.
#[test]
fn test_metadata_scan_matches_bruteforce_across_paths() {
    let (_dir, col) = setup_collection(4);
    col.create_index("cat")
        .expect("test: create secondary index");

    // 1_200 "hot" (> 1000 floor → cost model decides), 5 "cold" (< floor →
    // candidate path), remainder "other".
    let total: u64 = 2_000;
    let points: Vec<Point> = (0..total)
        .map(|id| {
            let cat = if id < 1_200 {
                "hot"
            } else if id < 1_205 {
                "cold"
            } else {
                "other"
            };
            make_point_with_payload(
                id,
                vec![1.0, 0.0, 0.0, 0.0],
                serde_json::json!({ "cat": cat }),
            )
        })
        .collect();
    col.upsert(points).expect("test: upsert");

    for target in ["hot", "cold", "other"] {
        let expected: std::collections::BTreeSet<u64> = (0..total)
            .filter(|&id| {
                let cat = if id < 1_200 {
                    "hot"
                } else if id < 1_205 {
                    "cold"
                } else {
                    "other"
                };
                cat == target
            })
            .collect();

        let filter = crate::filter::Filter::new(crate::filter::Condition::eq("cat", target));
        let cond = eq_string_cond("cat", target);
        let got = col.execute_scan_query(&filter, 10_000, Some(&cond));
        let got_ids: std::collections::BTreeSet<u64> = got.iter().map(|r| r.point.id).collect();

        assert_eq!(
            got_ids, expected,
            "metadata scan for cat={target} must return exactly the matching rows"
        );
    }
}

/// Under a small limit the full-scan branch (or the candidate branch) may be
/// selected; either way every returned row satisfies the predicate and the
/// limit is respected — the routing never leaks non-matching rows.
#[test]
fn test_metadata_scan_small_limit_is_correct() {
    let (_dir, col) = setup_collection(4);
    col.create_index("cat")
        .expect("test: create secondary index");

    let total: u64 = 2_000;
    let points: Vec<Point> = (0..total)
        .map(|id| {
            let cat = if id < 1_500 { "hot" } else { "other" };
            make_point_with_payload(
                id,
                vec![1.0, 0.0, 0.0, 0.0],
                serde_json::json!({ "cat": cat }),
            )
        })
        .collect();
    col.upsert(points).expect("test: upsert");

    let filter = crate::filter::Filter::new(crate::filter::Condition::eq("cat", "hot"));
    let cond = eq_string_cond("cat", "hot");
    let limit = 300;
    let got = col.execute_scan_query(&filter, limit, Some(&cond));

    assert_eq!(
        got.len(),
        limit,
        "limit must be respected (1_500 matches available)"
    );
    for r in &got {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("cat"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(
            cat,
            Some("hot"),
            "every returned row must match the predicate"
        );
    }
}
