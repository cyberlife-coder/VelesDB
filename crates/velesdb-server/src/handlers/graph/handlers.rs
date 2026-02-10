//! Graph HTTP handlers for VelesDB REST API.
//!
//! All graph operations delegate to `Collection` methods from `velesdb-core`.
//! The server is a thin HTTP layer — zero reimplemented graph logic.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use velesdb_core::collection::graph::GraphEdge;
use velesdb_core::Collection;

use crate::types::ErrorResponse;
use crate::AppState;

use super::types::{
    AddEdgeRequest, DegreeResponse, EdgeQueryParams, EdgeResponse, EdgesResponse,
    TraversalResultItem, TraversalStats, TraverseRequest, TraverseResponse,
};

/// Helper: get a collection or return 404.
fn get_collection_or_404(
    state: &AppState,
    name: &str,
) -> Result<Collection, (StatusCode, Json<ErrorResponse>)> {
    state.db.get_collection(name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{name}' not found"),
            }),
        )
    })
}

/// Adapter: convert core's node-ID paths to edge-ID paths.
///
/// Core's `TraversalResult.path` contains node IDs (including source).
/// The REST API contract returns edge IDs in paths.
/// For each consecutive pair of nodes, we look up the connecting edge.
pub(super) fn node_path_to_edge_ids(collection: &Collection, node_path: &[u64]) -> Vec<u64> {
    if node_path.len() < 2 {
        return Vec::new();
    }

    let mut edge_ids = Vec::with_capacity(node_path.len() - 1);
    for window in node_path.windows(2) {
        let source = window[0];
        let target = window[1];

        // Find the edge connecting source → target
        let outgoing = collection.get_outgoing_edges(source);
        if let Some(edge) = outgoing.iter().find(|e| e.target() == target) {
            edge_ids.push(edge.id());
        } else {
            // Reason: data race between traversal and edge removal; log and skip
            tracing::warn!(
                source,
                target,
                "Edge not found between consecutive traversal nodes — possible data race"
            );
        }
    }
    edge_ids
}

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
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<EdgeQueryParams>,
) -> Result<Json<EdgesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let label = params.label.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Query parameter 'label' is required. Listing all edges requires pagination (not yet implemented).".to_string(),
            }),
        )
    })?;

    let collection = get_collection_or_404(&state, &name)?;

    let edges: Vec<EdgeResponse> = tokio::task::spawn_blocking(move || -> Vec<EdgeResponse> {
        collection
            .get_edges_by_label(&label)
            .into_iter()
            .map(|e| EdgeResponse {
                id: e.id(),
                source: e.source(),
                target: e.target(),
                label: e.label().to_string(),
                properties: serde_json::to_value(e.properties()).unwrap_or_default(),
            })
            .collect()
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
            }),
        )
    })?;

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
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn add_edge(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
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

    let collection = get_collection_or_404(&state, &name)?;

    tokio::task::spawn_blocking(move || -> velesdb_core::Result<()> { collection.add_edge(edge) })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Task panicked: {e}"),
                }),
            )
        })?
        .map_err(|e| {
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
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn traverse_graph(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(request): Json<TraverseRequest>,
) -> Result<Json<TraverseResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate strategy before spawning blocking task
    let strategy = request.strategy.to_lowercase();
    if strategy != "bfs" && strategy != "dfs" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid strategy '{strategy}'. Use 'bfs' or 'dfs'."),
            }),
        ));
    }

    let collection = get_collection_or_404(&state, &name)?;

    let rel_type_strs: Vec<String> = request.rel_types.clone();
    let source = request.source;
    let max_depth = request.max_depth;
    let limit = request.limit;

    let results =
        tokio::task::spawn_blocking(move || -> velesdb_core::Result<Vec<TraversalResultItem>> {
            let rel_refs: Vec<&str> = rel_type_strs.iter().map(String::as_str).collect();
            let rel_types = if rel_refs.is_empty() {
                None
            } else {
                Some(rel_refs.as_slice())
            };

            let core_results = match strategy.as_str() {
                "bfs" => collection.traverse_bfs(source, max_depth, rel_types, limit),
                "dfs" => collection.traverse_dfs(source, max_depth, rel_types, limit),
                // Reason: validated above, unreachable
                _ => unreachable!(),
            };

            // Convert core TraversalResult → server TraversalResultItem with edge-ID paths
            core_results.map(|results| {
                results
                    .into_iter()
                    .map(|r| {
                        let edge_ids = node_path_to_edge_ids(&collection, &r.path);
                        TraversalResultItem {
                            target_id: r.target_id,
                            depth: r.depth,
                            path: edge_ids,
                        }
                    })
                    .collect()
            })
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Task panicked: {e}"),
                }),
            )
        })?
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
        next_cursor: None,
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
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_node_degree(
    State(state): State<Arc<AppState>>,
    Path((name, node_id)): Path<(String, u64)>,
) -> Result<Json<DegreeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let collection = get_collection_or_404(&state, &name)?;

    let (in_degree, out_degree) = tokio::task::spawn_blocking(move || -> (usize, usize) {
        collection.get_node_degree(node_id)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
            }),
        )
    })?;

    Ok(Json(DegreeResponse {
        in_degree,
        out_degree,
    }))
}
