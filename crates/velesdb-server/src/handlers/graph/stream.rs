//! SSE streaming graph traversal handler (EPIC-058 US-003).
//!
//! Provides Server-Sent Events endpoint for streaming graph traversal results.

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use super::service::GraphService;
use super::types::{
    StreamDoneEvent, StreamErrorEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams,
};

/// Stream graph traversal results via SSE.
///
/// Yields events:
/// - `node`: Each node reached during traversal
/// - `stats`: Periodic statistics (every 100 nodes)
/// - `done`: Traversal completed
/// - `error`: If an error occurs
#[allow(clippy::unused_async)]
pub async fn stream_traverse(
    State(graph_service): State<Arc<GraphService>>,
    Path(collection): Path<String>,
    Query(params): Query<StreamTraverseParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let start_time = Instant::now();

    // Parse relationship types
    let rel_types: Vec<String> = params
        .relationship_types
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    // Perform traversal (collect results since BfsIterator has lifetime)
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

    // Create SSE stream from results
    let stream = match traversal_result {
        Ok(results) => {
            let total = results.len();
            let mut max_depth: u32 = 0;

            // Build events vector
            let mut events: Vec<Result<Event, Infallible>> = Vec::with_capacity(total + 2);

            for (i, item) in results.into_iter().enumerate() {
                if item.depth > max_depth {
                    max_depth = item.depth;
                }

                // Node event
                let node_event = StreamNodeEvent {
                    id: item.target_id,
                    depth: item.depth,
                    path: item.path,
                };
                let event_data =
                    serde_json::to_string(&node_event).unwrap_or_else(|_| "{}".to_string());
                events.push(Ok(Event::default().event("node").data(event_data)));

                // Stats event every 100 nodes
                if (i + 1) % 100 == 0 {
                    let stats_event = StreamStatsEvent {
                        nodes_visited: i + 1,
                        // SAFETY: elapsed time in ms won't exceed u64::MAX (584M years)
                        elapsed_ms: start_time.elapsed().as_millis() as u64,
                    };
                    let stats_data =
                        serde_json::to_string(&stats_event).unwrap_or_else(|_| "{}".to_string());
                    events.push(Ok(Event::default().event("stats").data(stats_data)));
                }
            }

            // Done event
            let done_event = StreamDoneEvent {
                total_nodes: total,
                max_depth_reached: max_depth,
                // SAFETY: elapsed time in ms won't exceed u64::MAX
                elapsed_ms: start_time.elapsed().as_millis() as u64,
            };
            let done_data = serde_json::to_string(&done_event).unwrap_or_else(|_| "{}".to_string());
            events.push(Ok(Event::default().event("done").data(done_data)));

            stream::iter(events)
        }
        Err(e) => {
            // Error event
            let error_event = StreamErrorEvent { error: e };
            let error_data =
                serde_json::to_string(&error_event).unwrap_or_else(|_| "{}".to_string());
            stream::iter(vec![Ok(Event::default().event("error").data(error_data))])
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
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
}
