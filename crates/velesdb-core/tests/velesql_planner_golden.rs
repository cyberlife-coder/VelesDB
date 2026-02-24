#![cfg(feature = "persistence")]

use velesdb_core::velesql::{Parser, QueryPlan};

fn assert_plan_contains(sql: &str, expected_fragments: &[&str]) {
    let query = Parser::parse(sql).expect("query must parse");
    let plan = QueryPlan::from_select(&query.select);
    let tree = plan.to_tree();

    for fragment in expected_fragments {
        assert!(
            tree.contains(fragment),
            "golden plan mismatch for SQL: {sql}\nmissing fragment: {fragment}\nplan:\n{tree}"
        );
    }
}

#[test]
fn velesql_planner_golden_vector_with_filter() {
    assert_plan_contains(
        "SELECT * FROM docs WHERE vector NEAR $q AND category = 'tech' LIMIT 10",
        &[
            "Query Plan:",
            "VectorSearch",
            "Collection: docs",
            "Index used: HNSW",
            "Filter",
            "Limit: 10",
        ],
    );
}

#[test]
fn velesql_planner_golden_scan_with_offset() {
    assert_plan_contains(
        "SELECT * FROM docs LIMIT 5 OFFSET 20",
        &["Query Plan:", "TableScan: docs", "Limit: 5", "Offset: 20"],
    );
}
