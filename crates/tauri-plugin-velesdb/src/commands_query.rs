//! `VelesQL` query Tauri command extracted from `commands.rs` (EPIC-031 US-012).
//!
//! Contains the `query` and `read_query` commands and their
//! dispatch/aggregation helpers.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::state::VelesDbState;
use crate::types::{HybridResult, QueryRequest, QueryResponse};
use tauri::{command, AppHandle, Runtime, State};

/// Named `VelesQL` parameters, as [`QueryRequest::params`] carries them.
type Params = std::collections::HashMap<String, serde_json::Value>;

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

/// Executes a `VelesQL` statement that changes nothing — a SELECT, MATCH,
/// `SELECT EDGES`, `SHOW`, `DESCRIBE` or `EXPLAIN` — and refuses every other
/// one before it runs.
///
/// The command a read-only webview is granted: `query` runs DDL and DML as
/// well, so a `velesdb:read-only` window holding it could `DROP COLLECTION`.
#[command]
pub async fn read_query<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: QueryRequest,
) -> std::result::Result<QueryResponse, CommandError> {
    let start = std::time::Instant::now();
    let parsed = velesdb_core::velesql::Parser::parse(&request.query)
        .map_err(|e| Error::Database(e.into()))?;
    ensure_read_only(&parsed)?;
    let results = dispatch_tauri_query(&state, parsed, request).await?;
    Ok(QueryResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Refuses a statement that would change the database
/// ([`Query::is_read_only`](velesdb_core::velesql::Query::is_read_only)).
pub(crate) fn ensure_read_only(query: &velesdb_core::velesql::Query) -> crate::error::Result<()> {
    if query.is_read_only() {
        return Ok(());
    }
    Err(Error::Database(velesdb_core::Error::Query(
        "read_query runs only statements that change nothing (SELECT, MATCH, SELECT EDGES, \
         SHOW, DESCRIBE, EXPLAIN); use query for DDL and DML"
            .to_string(),
    )))
}

/// Dispatches a tauri query to aggregation or standard execution path.
async fn dispatch_tauri_query(
    state: &VelesDbState,
    parsed: velesdb_core::velesql::Query,
    request: QueryRequest,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    // `SelectStatement::is_aggregation_query` is the routing test every
    // surface shares: a vector GROUP BY runs inside `execute_query`.
    // The request's `collection` fills `_collection` for a top-level MATCH
    // when the caller set none: core's one rule, shared with REST `/query`.
    let params = velesdb_core::velesql::with_collection_param(
        &parsed,
        &request.params,
        request.collection.as_deref(),
    )
    .into_owned();
    if parsed.select.is_aggregation_query() && !parsed.select.from.is_empty() {
        return execute_tauri_aggregation(state, parsed, params).await;
    }

    state
        .run_db(move |db| {
            let search_results = db.execute_query(&parsed, &params)?;
            Ok(search_results
                .into_iter()
                .map(|r| search_result_to_hybrid(&r))
                .collect())
        })
        .await
        .map_err(CommandError::from)
}

/// Executes an aggregation query through `Database::execute_aggregate`, never a
/// collection handle: the database is where the observer's read gate runs, so
/// a denial refuses the aggregate and a scope narrows the rows it counts.
async fn execute_tauri_aggregation(
    state: &VelesDbState,
    parsed: velesdb_core::velesql::Query,
    params: Params,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    let agg_json = state
        .run_db(move |db| db.execute_aggregate(&parsed, &params).map_err(Error::from))
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
///
/// `vector_score` and `graph_score` are the components of those names core
/// tagged the row with, `None` when absent: core tags `vector_score` on every
/// row a vector scored, so a metadata, BM25-only or depth-ranked row reports
/// none instead of repeating its final score. `fused_score` is that score.
pub(crate) fn search_result_to_hybrid(r: &velesdb_core::SearchResult) -> HybridResult {
    HybridResult {
        node_id: r.point.id,
        vector_score: r.component_score("vector_score"),
        graph_score: r.component_score("graph_score"),
        fused_score: r.score,
        bindings: r.point.payload.clone(),
        column_data: None,
    }
}

#[cfg(test)]
#[path = "commands_query_tests.rs"]
mod tests;
