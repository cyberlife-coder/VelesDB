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
use velesdb_core::observer::QueryOperationKind;

use crate::handlers::helpers::run_blocking_typed;
use crate::types::ErrorResponse;
use crate::AppState;

use super::handlers::{graph_preamble, graph_read_preamble};
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
        ("edge_id" = String, Path, description = "Edge ID to remove (u64 as a string; precision-safe above 2^53-1)", pattern = "^[0-9]+$")
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
    // Edge removal takes write locks and persists — run it on the blocking pool.
    if run_blocking_typed(move || coll.remove_edge(edge_id)).await? {
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
    let coll = graph_read_preamble(&state, &name, QueryOperationKind::GraphTraversal)?;
    // Edge counting takes edge-store shard locks — run it on the blocking pool.
    let count = run_blocking_typed(move || coll.edge_count()).await?;
    Ok(Json(EdgeCountResponse { count }))
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
    let coll = graph_read_preamble(&state, &name, QueryOperationKind::GraphTraversal)?;
    // Node enumeration takes node-store locks — run it on the blocking pool.
    let node_ids = run_blocking_typed(move || coll.all_node_ids()).await?;
    let count = node_ids.len();
    Ok(Json(NodeListResponse { node_ids, count }))
}

/// Get edges for a specific node with direction filtering.
#[utoipa::path(
    get,
    path = "/collections/{name}/graph/nodes/{node_id}/edges",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("node_id" = String, Path, description = "Node ID (u64 as a string; precision-safe above 2^53-1)", pattern = "^[0-9]+$"),
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
    let coll = graph_read_preamble(&state, &name, QueryOperationKind::GraphTraversal)?;

    // Edge listing takes edge-store shard locks — run it on the blocking pool.
    let direction = params.direction.to_lowercase();
    let raw_edges = run_blocking_typed(move || match direction.as_str() {
        "in" => coll.get_incoming(node_id),
        "both" => {
            let mut all = coll.get_outgoing(node_id);
            all.extend(coll.get_incoming(node_id));
            all
        }
        _ => coll.get_outgoing(node_id),
    })
    .await?;

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
        ("node_id" = String, Path, description = "Node ID (u64 as a string; precision-safe above 2^53-1)", pattern = "^[0-9]+$")
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
    // Payload upsert takes write locks and persists — run it on the blocking pool.
    run_blocking_typed(move || coll.upsert_node_payload(node_id, &request.payload))
        .await?
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
        ("node_id" = String, Path, description = "Node ID (u64 as a string; precision-safe above 2^53-1)", pattern = "^[0-9]+$")
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
    let coll = graph_read_preamble(&state, &name, QueryOperationKind::GraphTraversal)?;
    // Payload lookup takes storage locks — run it on the blocking pool.
    let payload = run_blocking_typed(move || coll.get_node_payload(node_id))
        .await?
        .map_err(|e| {
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

    let coll = graph_read_preamble(&state, &name, QueryOperationKind::GraphTraversal)?;

    let limit = request.limit;
    let config = TraversalConfig::with_range(1, request.max_depth)
        .with_limit(limit)
        .with_rel_types(request.rel_types);

    // Parallel traversal is synchronous, rayon-dispatching core code — run
    // it on the blocking pool so the async workers stay responsive.
    let sources = request.sources;
    let raw_results =
        run_blocking_typed(move || coll.traverse_bfs_parallel(&sources, &config)).await?;

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
    let has_more = visited >= limit;

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

    // Gate the read (CORE-2). Graph embedding search has no metadata-filter
    // channel, so a denied or scope-narrowed decision refuses it (fail closed).
    match state.db.authorize_read(
        &name,
        velesdb_core::observer::QueryOperationKind::VectorSearch,
        None,
        None,
    ) {
        Ok(None) => {}
        Ok(Some(_)) | Err(_) => {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Read denied by governance policy".to_string(),
                    code: None,
                }),
            ));
        }
    }

    // Embedding search is CPU-bound, lock-taking core code — run it on the
    // blocking pool so the async workers stay responsive.
    let search_results =
        run_blocking_typed(move || coll.search_by_embedding(&request.vector, request.top_k))
            .await?
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
