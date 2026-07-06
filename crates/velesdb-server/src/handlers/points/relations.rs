//! Relation (graph edge) and TTL handlers for point-bearing collections.
//!
//! These endpoints work on **any** collection type (vector, graph, or metadata)
//! because edges live on the collection's embedded edge store, independently
//! of the payload/vector layer.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use velesdb_core::api_types::serde_id;
use velesdb_core::collection::graph::GraphEdge;
use velesdb_core::point::Point;

use crate::types::ErrorResponse;
use crate::AppState;

use super::super::helpers::{error_response, get_collection_or_404};

use velesdb_core::EXPIRES_AT_KEY;

/// Request body for `POST /collections/{name}/relations`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RelateRequest {
    /// Source point ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_input_schema))]
    pub source: u64,
    /// Target point ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_input_schema))]
    pub target: u64,
    /// Relationship type label (e.g. `"KNOWS"`, `"RELATED_TO"`).
    pub rel_type: String,
    /// Optional edge properties.
    #[serde(default)]
    pub properties: serde_json::Value,
}

/// Response body for `POST /collections/{name}/relations`.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelateResponse {
    /// Allocated edge ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub edge_id: u64,
}

/// A single relation edge in a response.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelationEdge {
    /// Edge ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub id: u64,
    /// Source point ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub source: u64,
    /// Target point ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub target: u64,
    /// Relationship type label.
    pub rel_type: String,
    /// Edge properties (null when empty).
    pub properties: serde_json::Value,
}

/// Response body for `GET /collections/{name}/points/{id}/relations`.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelationsResponse {
    /// Outgoing relation edges.
    pub edges: Vec<RelationEdge>,
    /// Total count.
    pub count: usize,
}

/// Request body for `PATCH /collections/{name}/points/{id}/ttl`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetTtlRequest {
    /// Number of seconds from now until this point expires.
    /// A value of `0` expires the point immediately.
    pub ttl_seconds: u64,
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

/// Create a relation edge between two points in a collection.
///
/// Works on vector, graph, and metadata collections alike.
/// The edge ID is auto-assigned; the response body carries the allocated value.
#[utoipa::path(
    post,
    path = "/collections/{name}/relations",
    params(("name" = String, Path, description = "Collection name")),
    request_body = RelateRequest,
    responses(
        (status = 201, description = "Relation created", body = RelateResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn relate_points(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<RelateRequest>,
) -> axum::response::Response {
    let coll = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let properties: std::collections::HashMap<String, serde_json::Value> = match req.properties {
        serde_json::Value::Object(ref map) => {
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
        serde_json::Value::Null => std::collections::HashMap::new(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "properties must be an object or null".to_string(),
            )
        }
    };

    match insert_edge_with_retry(&coll, &req, properties) {
        Ok(edge_id) => (StatusCode::CREATED, Json(RelateResponse { edge_id })).into_response(),
        Err(r) => r,
    }
}

/// Maximum collision retries before giving up on edge-ID allocation.
const MAX_EDGE_RETRIES: u32 = 1_000;

/// Assigns a collision-free edge ID and inserts the edge, retrying on `EdgeExists`.
///
/// Returns an `INTERNAL_SERVER_ERROR` response when [`MAX_EDGE_RETRIES`]
/// consecutive IDs are all taken — this indicates a corrupted ID-space seed and
/// should never occur in practice.
#[allow(clippy::result_large_err)]
fn insert_edge_with_retry(
    coll: &velesdb_core::collection::AnyCollection,
    req: &RelateRequest,
    properties: std::collections::HashMap<String, serde_json::Value>,
) -> Result<u64, axum::response::Response> {
    let mut next_id = coll.max_edge_id().map_or(1, |m| m.saturating_add(1));
    for _ in 0..MAX_EDGE_RETRIES {
        if coll.edge_exists(next_id) {
            next_id = next_id.saturating_add(1);
            continue;
        }
        let edge = match GraphEdge::new(next_id, req.source, req.target, &req.rel_type) {
            Ok(e) => e.with_properties(properties.clone()),
            Err(e) => {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    format!("invalid edge: {e}"),
                ))
            }
        };
        match coll.add_edge(edge) {
            Ok(()) => return Ok(next_id),
            Err(velesdb_core::Error::EdgeExists(_)) => {
                next_id = next_id.saturating_add(1);
            }
            Err(e) => {
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to create relation: {e}"),
                ))
            }
        }
    }
    Err(error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "edge id allocation exhausted after too many retries".to_string(),
    ))
}

/// Remove a relation edge by ID.
#[utoipa::path(
    delete,
    path = "/collections/{name}/relations/{edge_id}",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("edge_id" = String, Path, description = "Edge ID to remove (u64 as a string; precision-safe above 2^53-1)", pattern = "^[0-9]+$")
    ),
    responses(
        (status = 204, description = "Relation removed"),
        (status = 404, description = "Collection or edge not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn unrelate_points(
    Path((name, edge_id)): Path<(String, u64)>,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let coll = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(r) => return r,
    };

    if coll.remove_edge(edge_id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        let err = velesdb_core::Error::EdgeNotFound(edge_id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("{err} in collection '{name}'"),
                code: Some(err.code().to_string()),
            }),
        )
            .into_response()
    }
}

/// List outgoing relation edges for a point.
#[utoipa::path(
    get,
    path = "/collections/{name}/points/{id}/relations",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("id" = String, Path, description = "Point ID (u64 as a string; precision-safe above 2^53-1)", pattern = "^[0-9]+$")
    ),
    responses(
        (status = 200, description = "Outgoing relations", body = RelationsResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "graph"
)]
pub async fn get_point_relations(
    Path((name, id)): Path<(String, u64)>,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let coll = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let raw_edges = coll.get_outgoing_edges(id);
    let edges: Vec<RelationEdge> = raw_edges
        .into_iter()
        .map(|e| RelationEdge {
            id: e.id(),
            source: e.source(),
            target: e.target(),
            rel_type: e.label().to_string(),
            properties: serde_json::to_value(e.properties()).unwrap_or_default(),
        })
        .collect();

    let count = edges.len();
    Json(RelationsResponse { edges, count }).into_response()
}

/// Set (or refresh) the durable TTL of a point.
///
/// Persists `_veles_expires_at` in the point's payload so the expiry
/// survives a restart. A `ttl_seconds` of `0` expires the point immediately.
/// Expired points are excluded from all read surfaces (search/get/scroll/query);
/// refreshing an expired point returns 404; storage is reclaimed lazily.
#[utoipa::path(
    patch,
    path = "/collections/{name}/points/{id}/ttl",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("id" = String, Path, description = "Point ID (u64 as a string; precision-safe above 2^53-1)", pattern = "^[0-9]+$")
    ),
    request_body = SetTtlRequest,
    responses(
        (status = 204, description = "TTL set successfully"),
        (status = 400, description = "Non-object payload", body = ErrorResponse),
        (status = 404, description = "Collection or point not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "points"
)]
pub async fn set_point_ttl(
    Path((name, id)): Path<(String, u64)>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetTtlRequest>,
) -> axum::response::Response {
    let coll = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let point = match coll.get(&[id]).into_iter().flatten().next() {
        Some(p) => p,
        None => {
            let err = velesdb_core::Error::PointNotFound(id);
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("{err} in collection '{name}'"),
                    code: Some(err.code().to_string()),
                }),
            )
                .into_response();
        }
    };

    let expires_at = now_secs().saturating_add(req.ttl_seconds);
    let updated = match stamp_ttl(point, id, expires_at, &name) {
        Ok(p) => p,
        Err(r) => return r,
    };

    match coll.upsert(vec![updated]) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to set TTL: {e}"),
        ),
    }
}

/// Injects `_veles_expires_at` into a point's payload and returns an updated
/// [`Point`] ready for upsert. Returns an error response when the payload is
/// not a JSON object.
// axum::response::Response is intentionally large; this is the standard handler error type.
#[allow(clippy::result_large_err)]
fn stamp_ttl(
    point: Point,
    id: u64,
    expires_at: u64,
    collection: &str,
) -> Result<Point, axum::response::Response> {
    let mut payload = point
        .payload
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));

    let Some(obj) = payload.as_object_mut() else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("point {id} in '{collection}' has a non-object payload"),
        ));
    };

    obj.insert(
        EXPIRES_AT_KEY.to_string(),
        serde_json::Value::from(expires_at),
    );

    Ok(Point {
        id,
        vector: point.vector,
        payload: Some(payload),
        sparse_vectors: point.sparse_vectors,
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
