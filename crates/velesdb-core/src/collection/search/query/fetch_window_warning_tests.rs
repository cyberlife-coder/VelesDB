//! Candidate-ceiling warnings across graph filters and tie retries.

use super::super::FetchWindow;
use super::{fixture_row, query_params};
use crate::collection::search::query::{DispatchFinalizeArgs, QuerySearchOptions};
use crate::collection::Collection;
use crate::guardrails::{QueryContext, QueryLimits};
use crate::velesql::Parser;
use tempfile::TempDir;

fn hybrid_window_warnings<const CEILING: usize>(col: &Collection) -> usize {
    let sql = "SELECT * FROM docs AS d WHERE vector NEAR $q AND vector SPARSE_NEAR $s \
               AND MATCH (d)-[:CITES]->(other) ORDER BY similarity() DESC LIMIT 1";
    let query = Parser::parse(sql).expect("test: hybrid query");
    crate::velesql::QueryValidator::validate(&query).expect("test: valid SELECT");
    let params = query_params();
    let select = &query.select;
    let mut extracted =
        Collection::extract_query_components(select, &params).expect("test: query components");
    assert_eq!(
        col.select_fetch_window(
            select,
            &mut extracted,
            &params,
            1,
            &mut QuerySearchOptions::default()
        ),
        FetchWindow::Exhaustive
    );
    let sparse = extracted
        .sparse_vector_search
        .as_ref()
        .expect("test: sparse");
    let limits = QueryLimits::new().with_max_cardinality(1);
    let ctx = QueryContext::new(limits);
    window_warnings(1, || {
        col.dispatch_sparse_query::<CEILING>(select, &params, &extracted, sparse, CEILING, &ctx)
    })
}

fn window_warnings(
    winner: u64,
    run: impl FnOnce() -> crate::error::Result<Vec<crate::point::SearchResult>>,
) -> usize {
    let (results, warns) = crate::test_fixtures::fixtures::capture_warns(run);
    let rows = results.expect("only the returned row counts towards cardinality");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].point.id, winner);
    warns
        .iter()
        .filter(|message| message.contains("SELECT fetch stopped"))
        .count()
}

fn hybrid_graph_collection() -> (TempDir, Collection) {
    let (dir, col) = crate::test_fixtures::fixtures::setup_collection(2);
    col.upsert(vec![fixture_row(1), fixture_row(2)])
        .expect("test: rows");
    let edge = crate::collection::graph::GraphEdge::new(1, 1, 2, "CITES").expect("test: edge");
    col.add_edge(edge).expect("test: edge");
    (dir, col)
}

#[test]
fn a_full_hybrid_fetch_warns_before_the_graph_filters_its_rows() {
    let (_dir, col) = hybrid_graph_collection();
    assert_eq!(hybrid_window_warnings::<3>(&col), 0, "below the ceiling");
    assert_eq!(hybrid_window_warnings::<2>(&col), 1, "at the ceiling");
}

fn main_window_warnings(col: &Collection, sql: &str, winner: u64) -> usize {
    const CEILING: usize = 2;
    let query = Parser::parse(sql).expect("test: query");
    crate::velesql::QueryValidator::validate(&query).expect("test: valid SELECT");
    let params = query_params();
    let bound = col.bind_select(&query, &params).expect("test: bind");
    let query = bound.as_ref().unwrap_or(&query);
    let mut extracted = Collection::extract_query_components(&query.select, &params)
        .expect("test: query components");
    let mut search_opts = super::super::super::QuerySearchOptions::default();
    let fetch_limit = match col.select_fetch_window(
        &query.select,
        &mut extracted,
        &params,
        1,
        &mut search_opts,
    ) {
        FetchWindow::Bounded(rows) => rows,
        FetchWindow::Exhaustive => CEILING,
        FetchWindow::OrderedScan { .. } => {
            panic!("test: expected ranked dispatch")
        }
    };
    let ctx = QueryContext::new(QueryLimits::new().with_max_cardinality(1));
    let args = DispatchFinalizeArgs {
        extracted: &extracted,
        search_opts: &search_opts,
        limit: 1,
        fetch_limit,
        is_vgb: false,
        ctx: &ctx,
        started_at: ctx.select_started_at(),
    };
    window_warnings(winner, || {
        col.dispatch_and_finalize::<CEILING>(query, &params, &args)
    })
}

#[test]
fn a_full_text_fusion_warns_before_the_graph_filters_its_rows() {
    let (_dir, col) = hybrid_graph_collection();
    let sql = "SELECT * FROM docs AS d WHERE vector NEAR $q AND content MATCH 'banana' \
               AND NOT MATCH (d)-[:CITES]->(other) ORDER BY similarity() DESC LIMIT 1";
    assert_eq!(main_window_warnings(&col, sql, 2), 1);
}

#[test]
fn a_fetch_retried_for_tied_vectors_warns_only_once() {
    let (_dir, col) = crate::test_fixtures::fixtures::setup_collection(2);
    let points = [2, 1].map(|id| crate::point::Point::new(id, vec![1.0, 0.0], None));
    col.upsert(points).expect("test: tied vectors");
    let prefix = "SELECT * FROM docs WHERE vector NEAR $q ORDER BY similarity() DESC";
    let control = Parser::parse(&format!("{prefix} LIMIT 2")).expect("test: control");
    let tied = col
        .execute_query(&control, &query_params())
        .expect("test: ties");
    assert_eq!(tied.len(), 2);
    assert_eq!(tied[0].score.to_bits(), tied[1].score.to_bits());
    assert_eq!(
        main_window_warnings(&col, &format!("{prefix} LIMIT 1"), 1),
        1
    );
}
