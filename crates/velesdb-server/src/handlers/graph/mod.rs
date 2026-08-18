//! Graph handlers for VelesDB REST API.
//!
//! All graph operations route through `AppState.db.get_graph_collection()`.
//! Graph data persists on disk via `GraphCollection` / `GraphEngine`.
//! [EPIC-016/US-031]

pub mod handlers;
pub mod handlers_extended;
pub mod stream;
pub mod types;

// Re-export public API — original handlers
pub use handlers::{add_edge, add_edges_batch, get_edges, get_node_degree, traverse_graph};
// Re-export public API — extended handlers (parity)
pub use handlers_extended::{
    get_edge_count, get_node_edges, get_node_payload, graph_search, list_nodes, remove_edge,
    traverse_parallel, upsert_node_payload,
};
pub use stream::stream_traverse;
#[allow(unused_imports)]
pub use types::{
    AddEdgeRequest, AddEdgesBatchRequest, AddEdgesBatchResponse, DegreeResponse, EdgeCountResponse,
    EdgeQueryParams, EdgeResponse, EdgesResponse, GraphSearchRequest, GraphSearchResponse,
    GraphSearchResultItem, NodeEdgeQueryParams, NodeListResponse, NodePayloadResponse,
    ParallelTraverseRequest, StreamDoneEvent, StreamErrorEvent, StreamNodeEvent, StreamStatsEvent,
    StreamTraverseParams, TraversalResultItem, TraversalStats, TraverseRequest, TraverseResponse,
    UpsertNodePayloadRequest,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
