//! Graph HTTP handlers for VelesDB REST API.
//!
//! Provides endpoints for graph operations including edge queries, traversal, and degree.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use velesdb_core::collection::graph::GraphEdge;

use crate::types::ErrorResponse;

use super::service::GraphService;
use super::types::{
    AddEdgeRequest, DegreeResponse, EdgeQueryParams, EdgeResponse, EdgesResponse, TraversalStats,
    TraverseRequest, TraverseResponse,
};

/// Get edges from a collection's graph filtered by label.
///
/// Returns edges matching the specified label. The `label` query parameter is required.
///
/// # Errors
///
/// Returns an error tuple with status code and error response if the operation fails.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/edges",
    params(
        ("name" = String, Path, description = "Collection name"),
        EdgeQueryParams
    ),
    responses(
        (status = 200, description = "Edges retrieved successfully", body = EdgesResponse),
        (status = 400, description = "Missing required 'label' query parameter", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_edges(
    Path(name): Path<String>,
    Query(params): Query<EdgeQueryParams>,
    State(graph_service): State<GraphService>,
) -> Result<Json<EdgesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let label = params.label.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Query parameter 'label' is required. Listing all edges requires pagination (not yet implemented).".to_string(),
            }),
        )
    })?;

    let edges: Vec<EdgeResponse> = graph_service
        .get_edges_by_label(&name, &label)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to get edges: {e}"),
                }),
            )
        })?
        .into_iter()
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

/// Add an edge to a collection's graph.
///
/// # Errors
///
/// Returns an error tuple with status code and error response if:
/// - The request properties are invalid
/// - The edge creation fails
#[utoipa::path(
    post,
    path = "/collections/{name}/graph/edges",
    request_body = AddEdgeRequest,
    responses(
        (status = 201, description = "Edge added successfully"),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn add_edge(
    Path(name): Path<String>,
    State(graph_service): State<GraphService>,
    Json(request): Json<AddEdgeRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Convert properties from Value to HashMap<String, Value>
    let properties: std::collections::HashMap<String, serde_json::Value> = match request.properties
    {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        serde_json::Value::Null => std::collections::HashMap::new(),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Properties must be an object or null".to_string(),
                }),
            ));
        }
    };

    let edge = GraphEdge::new(request.id, request.source, request.target, &request.label)
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid edge: {e}"),
                }),
            )
        })?
        .with_properties(properties);

    graph_service.add_edge(&name, edge).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to add edge: {e}"),
            }),
        )
    })?;

    Ok(StatusCode::CREATED)
}

/// Traverse the graph using BFS or DFS from a source node.
///
/// # Errors
///
/// Returns an error tuple with status code and error response if traversal fails.
#[utoipa::path(
    post,
    path = "/collections/{name}/graph/traverse",
    request_body = TraverseRequest,
    responses(
        (status = 200, description = "Traversal completed successfully", body = TraverseResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn traverse_graph(
    Path(name): Path<String>,
    State(graph_service): State<GraphService>,
    Json(request): Json<TraverseRequest>,
) -> Result<Json<TraverseResponse>, (StatusCode, Json<ErrorResponse>)> {
    let results = match request.strategy.to_lowercase().as_str() {
        "bfs" => graph_service.traverse_bfs(
            &name,
            request.source,
            request.max_depth,
            request.limit,
            &request.rel_types,
        ),
        "dfs" => graph_service.traverse_dfs(
            &name,
            request.source,
            request.max_depth,
            request.limit,
            &request.rel_types,
        ),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid strategy '{}'. Use 'bfs' or 'dfs'.",
                        request.strategy
                    ),
                }),
            ));
        }
    }
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Traversal failed: {e}"),
            }),
        )
    })?;

    let depth_reached = results.iter().map(|r| r.depth).max().unwrap_or(0);
    let visited = results.len();
    let has_more = results.len() >= request.limit;

    Ok(Json(TraverseResponse {
        results,
        next_cursor: None, // Cursor pagination not implemented yet
        has_more,
        stats: TraversalStats {
            visited,
            depth_reached,
        },
    }))
}

/// Get the degree (in and out) of a specific node.
///
/// # Errors
///
/// Returns an error tuple with status code and error response if the query fails.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/nodes/{node_id}/degree",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("node_id" = u64, Path, description = "Node ID")
    ),
    responses(
        (status = 200, description = "Degree retrieved successfully", body = DegreeResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_node_degree(
    Path((name, node_id)): Path<(String, u64)>,
    State(graph_service): State<GraphService>,
) -> Result<Json<DegreeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (in_degree, out_degree) = graph_service.get_node_degree(&name, node_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get node degree: {e}"),
            }),
        )
    })?;

    Ok(Json(DegreeResponse {
        in_degree,
        out_degree,
    }))
}
