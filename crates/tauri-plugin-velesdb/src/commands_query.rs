//! `VelesQL` query Tauri command extracted from `commands.rs` (EPIC-031 US-012).
//!
//! Contains the `query` command and its dispatch/aggregation helpers.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::helpers::require_collection;
use crate::state::VelesDbState;
use crate::types::{HybridResult, QueryRequest, QueryResponse};
use tauri::{command, AppHandle, Runtime, State};

/// Executes a `VelesQL` query (EPIC-031 US-012).
///
/// Supports SELECT-style and MATCH-style `VelesQL` queries with vector
/// similarity search. Aggregation queries (GROUP BY, COUNT, etc.) are
/// auto-detected and routed to `execute_aggregate()`. DDL/DML/TRAIN/MATCH
/// queries are dispatched directly to `Database::execute_query`.
/// Returns results in `HybridResult` format.
#[command]
pub async fn query<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: QueryRequest,
) -> std::result::Result<QueryResponse, CommandError> {
    let start = std::time::Instant::now();

    // Parse the VelesQL query
    let parsed = velesdb_core::velesql::Parser::parse(&request.query)
        .map_err(|e| Error::Database(e.into()))?;

    let results = dispatch_tauri_query(&state, parsed, request).await?;

    Ok(QueryResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Dispatches a tauri query to aggregation or standard execution path.
async fn dispatch_tauri_query(
    state: &VelesDbState,
    parsed: velesdb_core::velesql::Query,
    request: QueryRequest,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    // `SelectStatement::is_aggregation_query` is the routing test every
    // surface shares: a vector GROUP BY runs inside `execute_query`.
    if parsed.select.is_aggregation_query() && !parsed.select.from.is_empty() {
        return execute_tauri_aggregation(state, parsed, request).await;
    }

    state
        .run_db(move |db| {
            let search_results = db.execute_query(&parsed, &request.params)?;
            Ok(search_results
                .into_iter()
                .map(|r| search_result_to_hybrid(&r))
                .collect())
        })
        .await
        .map_err(CommandError::from)
}

/// Executes an aggregation query through the collection API.
async fn execute_tauri_aggregation(
    state: &VelesDbState,
    parsed: velesdb_core::velesql::Query,
    request: QueryRequest,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    let agg_json = state
        .run_db(move |db| {
            let coll = require_collection(&db, &parsed.select.from)?;
            coll.execute_aggregate(&parsed, &request.params)
                .map_err(Error::from)
        })
        .await
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
pub(crate) fn search_result_to_hybrid(r: &velesdb_core::SearchResult) -> HybridResult {
    HybridResult {
        node_id: r.point.id,
        vector_score: Some(r.score),
        graph_score: None,
        fused_score: r.score,
        bindings: r.point.payload.clone(),
        column_data: None,
    }
}
