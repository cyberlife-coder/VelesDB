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
    AddEdgeRequest, DegreeResponse, EdgeCountResponse, EdgeQueryParams, EdgeResponse,
    EdgesResponse, GraphSearchRequest, GraphSearchResponse, GraphSearchResultItem,
    NodeEdgeQueryParams, NodeListResponse, NodePayloadResponse, ParallelTraverseRequest,
    TraversalStats, TraverseRequest, TraverseResponse, UpsertNodePayloadRequest,
};

/// Resolves a `GraphCollection` by name.
///
/// Returns 404 if no collection with that name exists at all.
/// Returns 409 if a collection exists but is not a graph collection (type mismatch).
/// Auto-creates a schemaless graph collection on first use if no collection exists yet,
/// preserving backward compatibility with workflows that drive graph ops without
/// an explicit `create_graph_collection` call.
#[allow(deprecated)]
pub(super) fn get_graph_collection_or_404(
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
                code: None,
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
                    code: None,
                }),
            )
        })?;

    state.db.get_graph_collection(name).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Graph collection '{}' not found after creation.", name),
                code: None,
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
                code: None,
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
                    code: None,
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
                    code: None,
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
                code: None,
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
                    code: None,
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
    let coll = get_graph_collection_or_404(&state, &name)?;
    if coll.remove_edge(edge_id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Edge {edge_id} not found in collection '{name}'"),
                code: None,
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
    let coll = get_graph_collection_or_404(&state, &name)?;
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
    let coll = get_graph_collection_or_404(&state, &name)?;
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
    let coll = get_graph_collection_or_404(&state, &name)?;

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
    let coll = get_graph_collection_or_404(&state, &name)?;
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
    let coll = get_graph_collection_or_404(&state, &name)?;
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

    let coll = get_graph_collection_or_404(&state, &name)?;

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
        next_cursor: None,
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
    let coll = get_graph_collection_or_404(&state, &name)?;

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
