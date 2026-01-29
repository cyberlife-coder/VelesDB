//! Knowledge Graph Tauri commands (EPIC-061/US-008 refactoring).
//!
//! Extracted from commands.rs to improve modularity.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::state::VelesDbState;
use crate::types::{
    AddEdgeRequest, EdgeOutput, GetEdgesRequest, GetNodeDegreeRequest, NodeDegreeOutput,
    TraversalOutput, TraverseGraphRequest,
};
use tauri::{command, AppHandle, Runtime, State};

/// Adds an edge to the knowledge graph.
#[command]
pub async fn add_edge<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: AddEdgeRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            // Convert properties to HashMap
            let properties: std::collections::HashMap<String, serde_json::Value> =
                match request.properties {
                    Some(serde_json::Value::Object(map)) => map.into_iter().collect(),
                    _ => std::collections::HashMap::new(),
                };

            let edge = velesdb_core::GraphEdge::new(
                request.id,
                request.source,
                request.target,
                &request.label,
            )
            .map_err(|e| Error::InvalidConfig(e.to_string()))?
            .with_properties(properties);

            coll.add_edge(edge)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Gets edges from the knowledge graph.
#[command]
pub async fn get_edges<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: GetEdgesRequest,
) -> std::result::Result<Vec<EdgeOutput>, CommandError> {
    state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let edges = if let Some(label) = &request.label {
                coll.get_edges_by_label(label)
            } else if let Some(source) = request.source {
                coll.get_outgoing_edges(source)
            } else if let Some(target) = request.target {
                coll.get_incoming_edges(target)
            } else {
                coll.get_all_edges()
            };

            Ok(edges
                .into_iter()
                .map(|e| EdgeOutput {
                    id: e.id(),
                    source: e.source(),
                    target: e.target(),
                    label: e.label().to_string(),
                    properties: serde_json::to_value(e.properties()).unwrap_or_default(),
                })
                .collect())
        })
        .map_err(CommandError::from)
}

/// Traverses the knowledge graph.
#[command]
pub async fn traverse_graph<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: TraverseGraphRequest,
) -> std::result::Result<Vec<TraversalOutput>, CommandError> {
    state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let rel_types: Option<Vec<&str>> = request
                .rel_types
                .as_ref()
                .map(|v| v.iter().map(String::as_str).collect());

            let results = if request.algorithm == "dfs" {
                coll.traverse_dfs(
                    request.source,
                    request.max_depth,
                    rel_types.as_deref(),
                    request.limit,
                )
            } else {
                coll.traverse_bfs(
                    request.source,
                    request.max_depth,
                    rel_types.as_deref(),
                    request.limit,
                )
            }
            .map_err(|e| Error::InvalidConfig(e.to_string()))?;

            Ok(results
                .into_iter()
                .map(|r| TraversalOutput {
                    target_id: r.target_id,
                    depth: r.depth,
                    path: r.path,
                })
                .collect())
        })
        .map_err(CommandError::from)
}

/// Gets the in-degree and out-degree of a node.
#[command]
pub async fn get_node_degree<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: GetNodeDegreeRequest,
) -> std::result::Result<NodeDegreeOutput, CommandError> {
    state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let (in_degree, out_degree) = coll.get_node_degree(request.node_id);

            Ok(NodeDegreeOutput {
                node_id: request.node_id,
                in_degree,
                out_degree,
            })
        })
        .map_err(CommandError::from)
}
