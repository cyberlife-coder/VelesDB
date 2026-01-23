//! Graph handlers for VelesDB REST API.
//!
//! Provides endpoints for graph operations including edge queries.
//! [EPIC-016/US-031]
//!
//! Note: Graph data is stored in a separate in-memory EdgeStore per collection.
//! This is managed by the GraphService state.

#![allow(dead_code)] // Handlers will be used when integrated into router

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use utoipa::{IntoParams, ToSchema};
use velesdb_core::collection::graph::{EdgeStore, GraphEdge};

use crate::types::ErrorResponse;

/// Shared graph service state for managing per-collection edge stores.
#[derive(Clone, Default)]
pub struct GraphService {
    stores: Arc<RwLock<HashMap<String, Arc<RwLock<EdgeStore>>>>>,
}

impl GraphService {
    /// Creates a new graph service.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Gets or creates an edge store for a collection.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    pub fn get_or_create_store(
        &self,
        collection_name: &str,
    ) -> Result<Arc<RwLock<EdgeStore>>, String> {
        let mut stores = self
            .stores
            .write()
            .map_err(|e| format!("Lock poisoned: {e}"))?;
        Ok(stores
            .entry(collection_name.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(EdgeStore::new())))
            .clone())
    }

    /// Adds an edge to a collection's graph.
    ///
    /// # Errors
    ///
    /// Returns an error if the lock is poisoned or if adding the edge fails.
    pub fn add_edge(&self, collection_name: &str, edge: GraphEdge) -> Result<(), String> {
        let store = self.get_or_create_store(collection_name)?;
        let mut guard = store.write().map_err(|e| format!("Lock error: {e}"))?;
        guard.add_edge(edge).map_err(|e| e.to_string())
    }

    /// Gets edges by label from a collection's graph.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    pub fn get_edges_by_label(
        &self,
        collection_name: &str,
        label: &str,
    ) -> Result<Vec<GraphEdge>, String> {
        let store = self.get_or_create_store(collection_name)?;
        let guard = store.read().map_err(|e| format!("Lock poisoned: {e}"))?;
        Ok(guard
            .get_edges_by_label(label)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Lists all stores (for metrics).
    ///
    /// # Errors
    ///
    /// Returns an error if the internal lock is poisoned.
    #[allow(clippy::type_complexity)]
    pub fn list_stores(&self) -> Result<Vec<(String, Arc<std::sync::RwLock<EdgeStore>>)>, String> {
        let stores = self
            .stores
            .read()
            .map_err(|e| format!("Lock poisoned: {e}"))?;
        Ok(stores.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }
}

/// Query parameters for edge filtering.
#[derive(Debug, Deserialize, IntoParams)]
pub struct EdgeQueryParams {
    /// Filter edges by label (e.g., "KNOWS", "FOLLOWS").
    #[param(example = "KNOWS")]
    pub label: Option<String>,
}

/// Response containing edges.
#[derive(Debug, Serialize, ToSchema)]
pub struct EdgesResponse {
    /// List of edges.
    pub edges: Vec<EdgeResponse>,
    /// Total count of edges returned.
    pub count: usize,
}

/// A single edge in the response.
#[derive(Debug, Serialize, ToSchema)]
pub struct EdgeResponse {
    /// Edge ID.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Edge label (relationship type).
    pub label: String,
    /// Edge properties.
    pub properties: serde_json::Value,
}

/// Request to add an edge to the graph.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddEdgeRequest {
    /// Edge ID.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Edge label (relationship type).
    pub label: String,
    /// Edge properties.
    #[serde(default)]
    pub properties: serde_json::Value,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_service_add_and_get() {
        let service = GraphService::new();
        let edge = GraphEdge::new(1, 100, 200, "KNOWS")
            .expect("valid edge")
            .with_properties(std::collections::HashMap::new());

        service
            .add_edge("test_collection", edge)
            .expect("should add");

        let edges = service
            .get_edges_by_label("test_collection", "KNOWS")
            .expect("should get edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].label(), "KNOWS");
    }

    #[test]
    fn test_edges_response_serialize() {
        let response = EdgesResponse {
            edges: vec![EdgeResponse {
                id: 1,
                source: 100,
                target: 200,
                label: "KNOWS".to_string(),
                properties: serde_json::json!({}),
            }],
            count: 1,
        };
        let json = serde_json::to_string(&response).expect("should serialize");
        assert!(json.contains("KNOWS"));
    }
}
