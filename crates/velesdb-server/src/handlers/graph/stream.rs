//! SSE streaming graph traversal handler (EPIC-058 US-003).
//!
//! Provides a Server-Sent Events endpoint for graph traversal results.
//!
//! # Streaming contract (finding F-17)
//!
//! The server-side traversal produced by `velesdb_core::GraphCollection::
//! traverse_bfs` / `traverse_dfs` is a synchronous call that returns a
//! fully materialised `Vec<TraversalResult>`. That means the "streaming"
//! semantics of this endpoint currently apply to the **wire level**
//! only — every node event is delivered to the client as an individual
//! SSE record (with back-pressure handled by axum's SSE wrapper), but
//! the traversal itself runs to completion before the first node
//! reaches the wire.
//!
//! F-17 of the pre-seed audit flagged this as a "fake SSE" problem.
//! Sprint 1 addresses the most damaging half of that finding: the
//! synchronous traversal is now executed on a `spawn_blocking` worker
//! so the async runtime stays responsive for concurrent requests,
//! and the SSE events are built off the runtime thread. True
//! incremental streaming — where each node is emitted to the wire
//! as soon as the traversal visits it — requires a new callback-based
//! core method (`traverse_bfs_stream(config, cb)`) and is tracked as
//! a post-seed EPIC in `docs/TECH_DEBT_REGISTRY.md`.

use axum::response::IntoResponse;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream;
use std::convert::Infallible;
use std::time::Instant;
use tracing::warn;

use std::sync::Arc;

use crate::types::ErrorResponse;
use crate::AppState;

use super::types::{
    StreamDoneEvent, StreamErrorEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams,
    TraversalResultItem,
};

/// Interval (in nodes) between periodic stats events.
const STATS_INTERVAL: usize = 100;

/// Stream graph traversal results via SSE.
///
/// The traversal runs to completion first, then its events are sent:
/// - `node`: each node reached
/// - `stats`: `nodes_visited` and `elapsed_ms`, after every 100th `node` event
/// - `done`: `total_nodes`, `max_depth_reached` and `elapsed_ms`, last
/// - `error`: sent alone instead, when the traversal worker fails
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/traverse/stream",
    tag = "graph",
    params(
        ("name" = String, Path, description = "Collection name"),
        StreamTraverseParams
    ),
    responses(
        (status = 200, description = "SSE stream of traversal events (node, stats, done, error)"),
        (status = 400, description = "Invalid algorithm (use 'bfs' or 'dfs'), a scope narrowing graph reads cannot apply (VELES-009), or a read the access gate denied with a client error (its own code)", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 409, description = "Collection exists but is not a graph collection", body = ErrorResponse)
    )
)]
pub async fn stream_traverse(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
    Query(params): Query<StreamTraverseParams>,
) -> axum::response::Response {
    let start_time = Instant::now();

    // Resolved and gated like every other graph read: a missing collection
    // answers 404 and a non-graph one 409, as `POST /graph/traverse` does,
    // rather than a 200 stream carrying only an `error` event; a denial or
    // a scope refuses the request (fail closed).
    let coll = match super::handlers::graph_read_preamble(
        &state,
        &collection,
        velesdb_core::observer::QueryOperationKind::GraphTraversal,
    ) {
        Ok(coll) => coll,
        Err(refusal) => return refusal.into_response(),
    };

    // An unknown algorithm must be rejected with the same 400 the
    // non-streaming traverse endpoint returns for its strategy, not silently
    // streamed as BFS (#2755).
    let use_bfs = match params.algorithm.to_lowercase().as_str() {
        "bfs" => true,
        "dfs" => false,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid algorithm '{}'. Use 'bfs' or 'dfs'.",
                        params.algorithm
                    ),
                    code: None,
                }),
            )
                .into_response();
        }
    };
    state.operational_metrics.record_graph_query();

    // F-17 fix: execute the synchronous traversal and event
    // construction on a blocking worker so the Tokio async runtime
    // stays responsive to other requests. The completed Vec<Event>
    // is then handed back to the async handler and fed to the SSE
    // wrapper as a lazy iterator, so individual events are still
    // pushed to the wire progressively under back-pressure.
    let events = tokio::task::spawn_blocking(move || {
        let results = run_traversal_blocking(&coll, use_bfs, &params);
        build_success_events(results, start_time)
    })
    .await
    .unwrap_or_else(|e| {
        // JoinError: panic or cancellation inside the blocking task.
        // Report it as an SSE error event so the client receives a
        // well-formed SSE frame instead of an aborted connection.
        build_error_events(format!(
            "Graph traversal worker failed: {e}. This is an internal \
             server error; please retry or contact the operator."
        ))
    });

    Sse::new(stream::iter(events))
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Runs the graph traversal synchronously on the calling (blocking)
/// thread. Factored out of the handler so it can be invoked from
/// `tokio::task::spawn_blocking` without leaking async internals.
fn run_traversal_blocking(
    coll: &velesdb_core::GraphCollection,
    use_bfs: bool,
    params: &StreamTraverseParams,
) -> Vec<TraversalResultItem> {
    use velesdb_core::collection::graph::TraversalConfig;

    let rel_types = parse_relationship_types(params.relationship_types.as_deref());

    let config = TraversalConfig::with_range(1, params.max_depth)
        .with_limit(params.limit)
        .with_rel_types(rel_types);

    let raw = if use_bfs {
        coll.traverse_bfs(params.start_node, &config)
    } else {
        coll.traverse_dfs(params.start_node, &config)
    };

    raw.into_iter()
        .map(|r| TraversalResultItem {
            target_id: r.target_id,
            depth: r.depth,
            path: r.path,
        })
        .collect()
}

/// Splits the comma-separated `relationship_types` query parameter.
///
/// Blank entries are dropped: core reads an empty list as "every type" but a
/// `""` entry as a type no edge carries, so an empty value
/// (`relationship_types=`) would otherwise filter out every edge and stream
/// an empty traversal instead of an unfiltered one.
fn parse_relationship_types(csv: Option<&str>) -> Vec<String> {
    csv.into_iter()
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Serializes an SSE event payload to JSON, logging a warning if serialization fails.
///
/// All current event types contain only primitives and are practically infallible.
/// The fallback exists as a safety net: it emits a well-formed JSON error object
/// rather than the misleading `"{}"` that would silently mask a real schema bug.
fn serialize_sse_event<T: serde::Serialize>(event: &T, event_type: &str) -> String {
    serde_json::to_string(event).unwrap_or_else(|e| {
        warn!(event_type, error = %e, "SSE event serialization failed; sending error fallback");
        r#"{"error":"internal: event serialization failed"}"#.to_string()
    })
}

fn build_success_events(
    results: Vec<TraversalResultItem>,
    start_time: Instant,
) -> Vec<Result<Event, Infallible>> {
    let total = results.len();
    let mut max_depth: u32 = 0;
    let mut events: Vec<Result<Event, Infallible>> = Vec::with_capacity(total + 2);

    for (i, item) in results.into_iter().enumerate() {
        if item.depth > max_depth {
            max_depth = item.depth;
        }

        let node_event = StreamNodeEvent {
            id: item.target_id,
            depth: item.depth,
            path: item.path,
        };
        let event_data = serialize_sse_event(&node_event, "node");
        events.push(Ok(Event::default().event("node").data(event_data)));

        if (i + 1) % STATS_INTERVAL == 0 {
            let stats_event = StreamStatsEvent {
                nodes_visited: i + 1,
                elapsed_ms: elapsed_ms(start_time),
            };
            let stats_data = serialize_sse_event(&stats_event, "stats");
            events.push(Ok(Event::default().event("stats").data(stats_data)));
        }
    }

    let done_event = StreamDoneEvent {
        total_nodes: total,
        max_depth_reached: max_depth,
        elapsed_ms: elapsed_ms(start_time),
    };
    let done_data = serialize_sse_event(&done_event, "done");
    events.push(Ok(Event::default().event("done").data(done_data)));

    events
}

fn build_error_events(error: String) -> Vec<Result<Event, Infallible>> {
    let error_event = StreamErrorEvent { error };
    let error_data = serialize_sse_event(&error_event, "error");
    vec![Ok(Event::default().event("error").data(error_data))]
}

/// Returns elapsed milliseconds since `start_time`.
///
/// The cast from `u128` to `u64` is safe because `u64::MAX` milliseconds
/// corresponds to ~584 million years, which no request will ever reach.
#[inline]
fn elapsed_ms(start_time: Instant) -> u64 {
    start_time.elapsed().as_millis() as u64
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
