//! Extended graph HTTP handlers for VelesDB REST API.
//!
//! Handlers added for API parity: remove_edge, edge_count, list_nodes,
//! node_edges, node_payload, parallel traversal, graph search.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use velesdb_core::collection::graph::TraversalConfig;

use crate::types::ErrorResponse;
use crate::AppState;

use super::handlers::graph_preamble;
use super::types::{
    EdgeCountResponse, EdgeResponse, EdgesResponse, GraphSearchRequest, GraphSearchResponse,
    GraphSearchResultItem, NodeEdgeQueryParams, NodeListResponse, NodePayloadResponse,
    ParallelTraverseRequest, TraversalStats, TraverseResponse, UpsertNodePayloadRequest,
};

/// Remove an edge by ID.
#[utoipa::path(
    delete,
    path = "/collections/{name}/graph/edges/{edge_id}",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("edge_id" = u64, Path, description = "Edge ID to remove")
    ),
    responses(
        (status = 204, description = "Edge removed successfully"),
        (status = 404, description = "Edge or collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn remove_edge(
    Path((name, edge_id)): Path<(String, u64)>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let coll = graph_preamble(&state, &name)?;
    if coll.remove_edge(edge_id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        // PR #586 Devin fix: emit `VELES-020 EdgeNotFound` with the
        // verbatim code so typed-error clients surface
        // `EdgeNotFoundError` instead of falling back to a status-
        // derived `'NOT_FOUND'` string. The error message retains the
        // collection context for operators reading server logs.
        let err = velesdb_core::Error::EdgeNotFound(edge_id);
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("{err} in collection '{name}'"),
                code: Some(err.code().to_string()),
            }),
        ))
    }
}

/// Get the total number of edges in the graph.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/edges/count",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Edge count retrieved", body = EdgeCountResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_edge_count(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<EdgeCountResponse>, (StatusCode, Json<ErrorResponse>)> {
    let coll = graph_preamble(&state, &name)?;
    Ok(Json(EdgeCountResponse {
        count: coll.edge_count(),
    }))
}

/// List all node IDs in the graph.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/nodes",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Node list retrieved", body = NodeListResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn list_nodes(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<NodeListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let coll = graph_preamble(&state, &name)?;
    let node_ids = coll.all_node_ids();
    let count = node_ids.len();
    Ok(Json(NodeListResponse { node_ids, count }))
}

/// Get edges for a specific node with direction filtering.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/nodes/{node_id}/edges",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("node_id" = u64, Path, description = "Node ID"),
        NodeEdgeQueryParams
    ),
    responses(
        (status = 200, description = "Node edges retrieved", body = EdgesResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_node_edges(
    Path((name, node_id)): Path<(String, u64)>,
    Query(params): Query<NodeEdgeQueryParams>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<EdgesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let coll = graph_preamble(&state, &name)?;

    let raw_edges = match params.direction.to_lowercase().as_str() {
        "in" => coll.get_incoming(node_id),
        "both" => {
            let mut all = coll.get_outgoing(node_id);
            all.extend(coll.get_incoming(node_id));
            all
        }
        _ => coll.get_outgoing(node_id),
    };

    let edges: Vec<EdgeResponse> = raw_edges
        .into_iter()
        .filter(|e| {
            params
                .label
                .as_ref()
                .is_none_or(|lbl| e.label() == lbl.as_str())
        })
        .map(|e| EdgeResponse {
            id: e.id(),
            source: e.source(),
            target: e.target(),
            label: e.label().to_string(),
            properties: serde_json::to_value(e.properties()).unwrap_or_default(),
        })
        .collect();

    let count = edges.len();
    Ok(Json(EdgesResponse { edges, count }))
}

/// Upsert a payload on a graph node.
#[utoipa::path(
    put,
    path = "/collections/{name}/graph/nodes/{node_id}/payload",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("node_id" = u64, Path, description = "Node ID")
    ),
    request_body = UpsertNodePayloadRequest,
    responses(
        (status = 204, description = "Payload stored successfully"),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn upsert_node_payload(
    Path((name, node_id)): Path<(String, u64)>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpsertNodePayloadRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let coll = graph_preamble(&state, &name)?;
    coll.upsert_node_payload(node_id, &request.payload)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to store payload: {e}"),
                    code: None,
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get the payload of a graph node.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/nodes/{node_id}/payload",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("node_id" = u64, Path, description = "Node ID")
    ),
    responses(
        (status = 200, description = "Payload retrieved", body = NodePayloadResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_node_payload(
    Path((name, node_id)): Path<(String, u64)>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<NodePayloadResponse>, (StatusCode, Json<ErrorResponse>)> {
    let coll = graph_preamble(&state, &name)?;
    let payload = coll.get_node_payload(node_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get payload: {e}"),
                code: None,
            }),
        )
    })?;
    Ok(Json(NodePayloadResponse { node_id, payload }))
}

/// Parallel multi-source BFS traversal.
#[utoipa::path(
    post,
    path = "/collections/{name}/graph/traverse/parallel",
    request_body = ParallelTraverseRequest,
    responses(
        (status = 200, description = "Parallel traversal completed", body = TraverseResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn traverse_parallel(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<ParallelTraverseRequest>,
) -> Result<Json<TraverseResponse>, (StatusCode, Json<ErrorResponse>)> {
    if request.sources.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "At least one source node ID is required".to_string(),
                code: None,
            }),
        ));
    }

    let coll = graph_preamble(&state, &name)?;

    let config = TraversalConfig::with_range(1, request.max_depth)
        .with_limit(request.limit)
        .with_rel_types(request.rel_types);

    let raw_results = coll.traverse_bfs_parallel(&request.sources, &config);

    let results: Vec<super::types::TraversalResultItem> = raw_results
        .into_iter()
        .map(|r| super::types::TraversalResultItem {
            target_id: r.target_id,
            depth: r.depth,
            path: r.path,
        })
        .collect();

    let depth_reached = results.iter().map(|r| r.depth).max().unwrap_or(0);
    let visited = results.len();
    let has_more = visited >= request.limit;

    Ok(Json(TraverseResponse {
        results,
        has_more,
        stats: TraversalStats {
            visited,
            depth_reached,
        },
    }))
}

/// Search graph nodes by embedding similarity.
#[utoipa::path(
    post,
    path = "/collections/{name}/graph/search",
    request_body = GraphSearchRequest,
    responses(
        (status = 200, description = "Graph search results", body = GraphSearchResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn graph_search(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<GraphSearchRequest>,
) -> Result<Json<GraphSearchResponse>, (StatusCode, Json<ErrorResponse>)> {
    let coll = graph_preamble(&state, &name)?;

    if !coll.has_embeddings() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Graph collection '{name}' does not have embeddings. \
                     Create it with create_graph_collection_with_embeddings() to enable search."
                ),
                code: None,
            }),
        ));
    }

    let search_results = coll
        .search_by_embedding(&request.vector, request.top_k)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Graph search failed: {e}"),
                    code: None,
                }),
            )
        })?;

    let results: Vec<GraphSearchResultItem> = search_results
        .into_iter()
        .map(|r| GraphSearchResultItem {
            id: r.point.id,
            score: r.score,
            payload: r.point.payload,
        })
        .collect();

    Ok(Json(GraphSearchResponse { results }))
}
