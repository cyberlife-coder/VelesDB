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
//! a post-seed EPIC in `docs/ARCHITECTURE.md`.

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream};
use std::convert::Infallible;
use std::time::Instant;

use std::sync::Arc;

use crate::AppState;

use super::types::{
    StreamDoneEvent, StreamErrorEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams,
    TraversalResultItem,
};

/// Interval (in nodes) between periodic stats events.
const STATS_INTERVAL: usize = 100;

/// Stream graph traversal results via SSE.
///
/// Yields events:
/// - `node`: Each node reached during traversal
/// - `stats`: Periodic statistics (every [`STATS_INTERVAL`] nodes)
/// - `done`: Traversal completed
/// - `error`: If an error occurs
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/traverse/stream",
    tag = "graph",
    params(
        ("name" = String, Path, description = "Collection name"),
        StreamTraverseParams
    ),
    responses(
        (status = 200, description = "SSE stream of traversal events (node, stats, done, error)")
    )
)]
pub async fn stream_traverse(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
    Query(params): Query<StreamTraverseParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let start_time = Instant::now();

    // Snapshot the parameters and state needed by the blocking worker.
    let coll_handle = state.db.get_graph_collection(&collection);

    // F-17 fix: execute the synchronous traversal and event
    // construction on a blocking worker so the Tokio async runtime
    // stays responsive to other requests. The completed Vec<Event>
    // is then handed back to the async handler and fed to the SSE
    // wrapper as a lazy iterator, so individual events are still
    // pushed to the wire progressively under back-pressure.
    let events = tokio::task::spawn_blocking(move || {
        let traversal_result = run_traversal_blocking(coll_handle, collection.as_str(), &params);
        build_sse_events(traversal_result, start_time)
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

    Sse::new(stream::iter(events)).keep_alive(KeepAlive::default())
}

/// Runs the graph traversal synchronously on the calling (blocking)
/// thread. Factored out of the handler so it can be invoked from
/// `tokio::task::spawn_blocking` without leaking async internals.
fn run_traversal_blocking(
    coll_handle: Option<velesdb_core::GraphCollection>,
    collection_name: &str,
    params: &StreamTraverseParams,
) -> Result<Vec<TraversalResultItem>, String> {
    use velesdb_core::collection::graph::TraversalConfig;

    let Some(coll) = coll_handle else {
        return Err(format!(
            "Collection '{collection_name}' not found or is not a graph collection."
        ));
    };

    let rel_types: Vec<String> = params
        .relationship_types
        .as_ref()
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let config = TraversalConfig::with_range(1, params.max_depth)
        .with_limit(params.limit)
        .with_rel_types(rel_types);

    let raw = match params.algorithm.to_lowercase().as_str() {
        "dfs" => coll.traverse_dfs(params.start_node, &config),
        _ => coll.traverse_bfs(params.start_node, &config),
    };

    Ok(raw
        .into_iter()
        .map(|r| TraversalResultItem {
            target_id: r.target_id,
            depth: r.depth,
            path: r.path,
        })
        .collect())
}

/// Converts a traversal result into a sequence of SSE events.
///
/// Extracted to keep the handler thin and the logic testable.
fn build_sse_events(
    traversal_result: Result<Vec<TraversalResultItem>, String>,
    start_time: Instant,
) -> Vec<Result<Event, Infallible>> {
    match traversal_result {
        Ok(results) => build_success_events(results, start_time),
        Err(e) => build_error_events(e),
    }
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
        let event_data = serde_json::to_string(&node_event).unwrap_or_else(|_| "{}".to_string());
        events.push(Ok(Event::default().event("node").data(event_data)));

        if (i + 1) % STATS_INTERVAL == 0 {
            let stats_event = StreamStatsEvent {
                nodes_visited: i + 1,
                elapsed_ms: elapsed_ms(start_time),
            };
            let stats_data =
                serde_json::to_string(&stats_event).unwrap_or_else(|_| "{}".to_string());
            events.push(Ok(Event::default().event("stats").data(stats_data)));
        }
    }

    let done_event = StreamDoneEvent {
        total_nodes: total,
        max_depth_reached: max_depth,
        elapsed_ms: elapsed_ms(start_time),
    };
    let done_data = serde_json::to_string(&done_event).unwrap_or_else(|_| "{}".to_string());
    events.push(Ok(Event::default().event("done").data(done_data)));

    events
}

fn build_error_events(error: String) -> Vec<Result<Event, Infallible>> {
    let error_event = StreamErrorEvent { error };
    let error_data = serde_json::to_string(&error_event).unwrap_or_else(|_| "{}".to_string());
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
mod tests {
    use super::*;

    #[test]
    fn test_stream_node_event_serialize() {
        let event = StreamNodeEvent {
            id: 123,
            depth: 2,
            path: vec![1, 2],
        };
        let json = serde_json::to_string(&event).expect("should serialize");
        assert!(json.contains("123"));
        assert!(json.contains("\"depth\":2"));
    }

    #[test]
    fn test_stream_done_event_serialize() {
        let event = StreamDoneEvent {
            total_nodes: 100,
            max_depth_reached: 5,
            elapsed_ms: 150,
        };
        let json = serde_json::to_string(&event).expect("should serialize");
        assert!(json.contains("100"));
        assert!(json.contains("max_depth_reached"));
    }

    #[test]
    fn test_stream_error_event_serialize() {
        let event = StreamErrorEvent {
            error: "Collection not found".to_string(),
        };
        let json = serde_json::to_string(&event).expect("should serialize");
        assert!(json.contains("Collection not found"));
    }

    #[test]
    fn test_build_error_events_returns_single_error() {
        let events = build_error_events("test error".to_string());
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_elapsed_ms_returns_reasonable_value() {
        let start = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let ms = elapsed_ms(start);
        assert!(ms >= 5, "elapsed should be at least 5ms, got {ms}");
    }
}
