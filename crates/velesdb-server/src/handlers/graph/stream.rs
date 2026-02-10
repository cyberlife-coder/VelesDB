//! SSE streaming graph traversal handler.
//!
//! Provides Server-Sent Events endpoint for streaming graph traversal results.
//! Delegates to `Collection` traversal methods from `velesdb-core`.

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use crate::AppState;

use super::handlers::node_path_to_edge_ids;
use super::types::{
    StreamDoneEvent, StreamErrorEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams,
};

/// (target_id, depth, edge_id_path) — intermediate result from spawn_blocking traversal.
type TraversalTuple = (u64, u32, Vec<u64>);

/// Stream graph traversal results via SSE.
///
/// Yields events:
/// - `node`: Each node reached during traversal
/// - `stats`: Periodic statistics (every 100 nodes)
/// - `done`: Traversal completed
/// - `error`: If an error occurs
pub async fn stream_traverse(
    State(state): State<Arc<AppState>>,
    Path(collection_name): Path<String>,
    Query(params): Query<StreamTraverseParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let start_time = Instant::now();

    // Parse relationship types
    let rel_types: Vec<String> = params
        .relationship_types
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let algorithm = params.algorithm.to_lowercase();
    let start_node = params.start_node;
    let max_depth = params.max_depth;
    let limit = params.limit;

    // Get collection — if not found, emit error event
    let collection = match state.db.get_collection(&collection_name) {
        Some(c) => c,
        None => {
            let error_event = StreamErrorEvent {
                error: format!("Collection '{collection_name}' not found"),
            };
            let error_data =
                serde_json::to_string(&error_event).unwrap_or_else(|_| "{}".to_string());
            let events = vec![Ok(Event::default().event("error").data(error_data))];
            return Sse::new(stream::iter(events)).keep_alive(KeepAlive::default());
        }
    };

    // Perform traversal in blocking task (CPU-bound)
    let traversal_result: Result<velesdb_core::Result<Vec<TraversalTuple>>, _> =
        tokio::task::spawn_blocking(move || -> velesdb_core::Result<Vec<TraversalTuple>> {
            let rel_refs: Vec<&str> = rel_types.iter().map(String::as_str).collect();
            let rel_types_opt = if rel_refs.is_empty() {
                None
            } else {
                Some(rel_refs.as_slice())
            };

            let core_results = match algorithm.as_str() {
                "dfs" => collection.traverse_dfs(start_node, max_depth, rel_types_opt, limit),
                _ => collection.traverse_bfs(start_node, max_depth, rel_types_opt, limit),
            };

            // Convert to edge-ID paths for API compatibility
            core_results.map(|results| {
                results
                    .into_iter()
                    .map(|r| {
                        let edge_ids = node_path_to_edge_ids(&collection, &r.path);
                        (r.target_id, r.depth, edge_ids)
                    })
                    .collect::<Vec<_>>()
            })
        })
        .await;

    // Build SSE events from results
    let stream = match traversal_result {
        Ok(Ok(results)) => {
            let total = results.len();
            let mut max_depth_val: u32 = 0;
            let mut events: Vec<Result<Event, Infallible>> = Vec::with_capacity(total + 2);

            for (i, (target_id, depth, path)) in results.into_iter().enumerate() {
                if depth > max_depth_val {
                    max_depth_val = depth;
                }

                let node_event = StreamNodeEvent {
                    id: target_id,
                    depth,
                    path,
                };
                let event_data =
                    serde_json::to_string(&node_event).unwrap_or_else(|_| "{}".to_string());
                events.push(Ok(Event::default().event("node").data(event_data)));

                // Stats event every 100 nodes
                if (i + 1) % 100 == 0 {
                    let stats_event = StreamStatsEvent {
                        nodes_visited: i + 1,
                        #[allow(clippy::cast_possible_truncation)]
                        // Reason: elapsed time in ms won't exceed u64::MAX (584M years)
                        elapsed_ms: start_time.elapsed().as_millis() as u64,
                    };
                    let stats_data =
                        serde_json::to_string(&stats_event).unwrap_or_else(|_| "{}".to_string());
                    events.push(Ok(Event::default().event("stats").data(stats_data)));
                }
            }

            let done_event = StreamDoneEvent {
                total_nodes: total,
                max_depth_reached: max_depth_val,
                #[allow(clippy::cast_possible_truncation)]
                // Reason: elapsed time in ms won't exceed u64::MAX
                elapsed_ms: start_time.elapsed().as_millis() as u64,
            };
            let done_data = serde_json::to_string(&done_event).unwrap_or_else(|_| "{}".to_string());
            events.push(Ok(Event::default().event("done").data(done_data)));

            stream::iter(events)
        }
        Ok(Err(e)) => {
            let error_event = StreamErrorEvent {
                error: e.to_string(),
            };
            let error_data =
                serde_json::to_string(&error_event).unwrap_or_else(|_| "{}".to_string());
            stream::iter(vec![Ok(Event::default().event("error").data(error_data))])
        }
        Err(e) => {
            let error_event = StreamErrorEvent {
                error: format!("Task panicked: {e}"),
            };
            let error_data =
                serde_json::to_string(&error_event).unwrap_or_else(|_| "{}".to_string());
            stream::iter(vec![Ok(Event::default().event("error").data(error_data))])
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
