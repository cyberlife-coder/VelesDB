//! Graph HTTP handlers for VelesDB REST API.
//!
//! All graph operations are routed through `AppState.db.get_graph_collection()`.
//! No separate GraphService state — graph data persists via GraphCollection/GraphEngine.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use velesdb_core::collection::graph::{GraphEdge, TraversalConfig};

use crate::types::ErrorResponse;
use crate::AppState;

use super::types::{
    AddEdgeRequest, DegreeResponse, EdgeQueryParams, EdgeResponse, EdgesResponse, TraversalStats,
    TraverseRequest, TraverseResponse,
};

/// Resolves a `GraphCollection` by name.
///
/// Returns 404 if no collection with that name exists at all.
/// Returns 409 if a collection exists but is not a graph collection (type mismatch).
/// Auto-creates a schemaless graph collection on first use if no collection exists yet,
/// preserving backward compatibility with workflows that drive graph ops without
/// an explicit `create_graph_collection` call.
#[allow(deprecated)]
fn get_graph_collection_or_404(
    state: &AppState,
    name: &str,
) -> Result<velesdb_core::GraphCollection, (StatusCode, Json<ErrorResponse>)> {
    // Fast path: already registered as a graph collection.
    if let Some(c) = state.db.get_graph_collection(name) {
        return Ok(c);
    }

    // Check if a collection with this name exists but with a different type.
    // Attempting to create over it would return CollectionExists — surface as 409.
    if state.db.get_collection(name).is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: format!(
                    "Collection '{}' exists but is not a graph collection. \
                     Use /collections/{}/graph only on graph-typed collections.",
                    name, name
                ),
            }),
        ));
    }

    // No collection at all — auto-create a schemaless graph collection.
    use velesdb_core::GraphSchema;
    state
        .db
        .create_graph_collection(name, GraphSchema::schemaless())
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to auto-create graph collection '{}': {e}", name),
                }),
            )
        })?;

    state.db.get_graph_collection(name).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Graph collection '{}' not found after creation.", name),
            }),
        )
    })
}

/// Get edges from a collection's graph filtered by label.
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
    State(state): State<Arc<AppState>>,
) -> Result<Json<EdgesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let label = params.label.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Query parameter 'label' is required. Listing all edges requires pagination (not yet implemented).".to_string(),
            }),
        )
    })?;

    let coll = get_graph_collection_or_404(&state, &name)?;

    let edges: Vec<EdgeResponse> = coll
        .get_edges(Some(&label))
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
#[utoipa::path(
    post,
    path = "/collections/{name}/graph/edges",
    request_body = AddEdgeRequest,
    responses(
        (status = 201, description = "Edge added successfully"),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn add_edge(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddEdgeRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
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

    let coll = get_graph_collection_or_404(&state, &name)?;

    coll.add_edge(edge).map_err(|e| {
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
#[utoipa::path(
    post,
    path = "/collections/{name}/graph/traverse",
    request_body = TraverseRequest,
    responses(
        (status = 200, description = "Traversal completed successfully", body = TraverseResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn traverse_graph(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<TraverseRequest>,
) -> Result<Json<TraverseResponse>, (StatusCode, Json<ErrorResponse>)> {
    let coll = get_graph_collection_or_404(&state, &name)?;

    let config = TraversalConfig::with_range(1, request.max_depth)
        .with_limit(request.limit)
        .with_rel_types(request.rel_types);

    let raw_results = match request.strategy.to_lowercase().as_str() {
        "bfs" => coll.traverse_bfs(request.source, &config),
        "dfs" => coll.traverse_dfs(request.source, &config),
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
    };

    // Convert TraversalResult -> TraversalResultItem
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
        next_cursor: None,
        has_more,
        stats: TraversalStats {
            visited,
            depth_reached,
        },
    }))
}

/// Get the degree (in and out) of a specific node.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/nodes/{node_id}/degree",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("node_id" = u64, Path, description = "Node ID")
    ),
    responses(
        (status = 200, description = "Degree retrieved successfully", body = DegreeResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_node_degree(
    Path((name, node_id)): Path<(String, u64)>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<DegreeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let coll = get_graph_collection_or_404(&state, &name)?;
    let (in_degree, out_degree) = coll.node_degree(node_id);
    Ok(Json(DegreeResponse {
        in_degree,
        out_degree,
    }))
}
