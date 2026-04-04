//! `VelesQL` query Tauri command extracted from `commands.rs` (EPIC-031 US-012).
//!
//! Contains the `query` command and its dispatch/aggregation helpers.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::helpers::require_collection;
use crate::state::VelesDbState;
use crate::types::{HybridResult, QueryRequest, QueryResponse};
use tauri::{command, AppHandle, Runtime, State};
use velesdb_core::velesql::SelectColumns;

/// Detects aggregation queries (COUNT, SUM, AVG, etc. in SELECT).
fn is_aggregation_query(parsed: &velesdb_core::velesql::Query) -> bool {
    match &parsed.select.columns {
        SelectColumns::Aggregations(_) => true,
        SelectColumns::Mixed { aggregations, .. } => !aggregations.is_empty(),
        _ => false,
    }
}

/// Executes a `VelesQL` query (EPIC-031 US-012).
///
/// Supports SELECT-style `VelesQL` queries with vector similarity search.
/// Aggregation queries (GROUP BY, COUNT, etc.) are auto-detected and routed
/// to `execute_aggregate()`. DDL/DML/TRAIN queries are dispatched directly
/// to `Database::execute_query`. MATCH queries are not yet supported through
/// this endpoint. Returns results in `HybridResult` format.
#[allow(clippy::too_many_lines)]
#[command]
pub async fn query<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: QueryRequest,
) -> std::result::Result<QueryResponse, CommandError> {
    let start = std::time::Instant::now();

    // Parse the VelesQL query
    let parsed = velesdb_core::velesql::Parser::parse(&request.query)
        .map_err(|e| Error::InvalidConfig(format!("VelesQL parse error: {}", e.message)))?;

    // MATCH queries are not supported through this endpoint.
    if parsed.is_match_query() {
        return Err(CommandError::from(Error::InvalidConfig(
            "MATCH queries are not supported through the query endpoint. \
             Use graph-specific commands instead."
                .to_string(),
        )));
    }

    let results = dispatch_tauri_query(&state, &parsed, &request)?;

    Ok(QueryResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Dispatches a tauri query to aggregation or standard execution path.
fn dispatch_tauri_query(
    state: &VelesDbState,
    parsed: &velesdb_core::velesql::Query,
    request: &QueryRequest,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    let collection_name = &parsed.select.from;

    if is_aggregation_query(parsed) && !collection_name.is_empty() {
        return execute_tauri_aggregation(state, parsed, request, collection_name);
    }

    state
        .with_db(|db| {
            let search_results = db.execute_query(parsed, &request.params)?;
            Ok(search_results
                .into_iter()
                .map(|r| search_result_to_hybrid(&r))
                .collect())
        })
        .map_err(CommandError::from)
}

/// Executes an aggregation query through the collection API.
fn execute_tauri_aggregation(
    state: &VelesDbState,
    parsed: &velesdb_core::velesql::Query,
    request: &QueryRequest,
    collection_name: &str,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    let agg_json = state
        .with_db(|db| {
            let coll = require_collection(&db, collection_name)?;
            coll.execute_aggregate(parsed, &request.params)
                .map_err(|e| Error::InvalidConfig(format!("Aggregation error: {e}")))
        })
        .map_err(CommandError::from)?;

    Ok(vec![HybridResult {
        node_id: 0,
        vector_score: None,
        graph_score: None,
        fused_score: 0.0,
        bindings: None,
        column_data: Some(agg_json),
    }])
}

/// Converts a `SearchResult` to a `HybridResult`.
fn search_result_to_hybrid(r: &velesdb_core::SearchResult) -> HybridResult {
    HybridResult {
        node_id: r.point.id,
        vector_score: Some(r.score),
        graph_score: None,
        fused_score: r.score,
        bindings: r.point.payload.clone(),
        column_data: None,
    }
}
