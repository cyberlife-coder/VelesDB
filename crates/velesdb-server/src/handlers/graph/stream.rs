//! SSE streaming graph traversal handler (EPIC-058 US-003).
//!
//! Provides a Server-Sent Events endpoint for streaming graph traversal
//! results incrementally, avoiding full buffering for large traversals.

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream};
use std::convert::Infallible;
use std::time::Instant;

use super::service::GraphService;
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
#[allow(clippy::unused_async)]
pub async fn stream_traverse(
    State(graph_service): State<GraphService>,
    Path(collection): Path<String>,
    Query(params): Query<StreamTraverseParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let start_time = Instant::now();

    let rel_types: Vec<String> = params
        .relationship_types
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let traversal_result = match params.algorithm.to_lowercase().as_str() {
        "dfs" => graph_service.traverse_dfs(
            &collection,
            params.start_node,
            params.max_depth,
            params.limit,
            &rel_types,
        ),
        _ => graph_service.traverse_bfs(
            &collection,
            params.start_node,
            params.max_depth,
            params.limit,
            &rel_types,
        ),
    };

    let events = build_sse_events(traversal_result, start_time);
    Sse::new(stream::iter(events)).keep_alive(KeepAlive::default())
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
