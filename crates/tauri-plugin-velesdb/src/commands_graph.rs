//! Knowledge Graph Tauri commands (EPIC-061/US-008 refactoring).
//!
//! Extracted from commands.rs to improve modularity.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::events::emit_collection_created;
use crate::helpers::{parse_metric, require_graph_collection};
use crate::state::VelesDbState;
use crate::types::{
    AddEdgeRequest, CollectionInfo, CreateGraphCollectionRequest, EdgeOutput, GetEdgesRequest,
    GetNodeDegreeRequest, NodeDegreeOutput, TraversalOutput, TraverseGraphParallelRequest,
    TraverseGraphRequest,
};
use tauri::{command, AppHandle, Runtime, State};
use velesdb_core::collection::graph::TraversalConfig;
use velesdb_core::GraphSchema;

/// Creates a graph collection with optional schema and embeddings.
///
/// When `graph_schema` is provided, it is deserialized into a [`GraphSchema`].
/// When omitted, a schemaless graph is created. If `dimension` is set, node
/// embeddings are enabled with the given metric.
#[command]
pub async fn create_graph_collection<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: CreateGraphCollectionRequest,
) -> std::result::Result<CollectionInfo, CommandError> {
    let schema = match &request.graph_schema {
        Some(json_val) => serde_json::from_value::<GraphSchema>(json_val.clone())
            .map_err(|e| Error::InvalidConfig(format!("Invalid graph schema: {e}")))?,
        None => GraphSchema::schemaless(),
    };

    let result = state
        .with_db(|db| {
            if let Some(dim) = request.dimension {
                let metric = parse_metric(&request.metric)?;
                db.create_graph_collection_with_embeddings(&request.name, schema, dim, metric)?;
            } else {
                db.create_graph_collection(&request.name, schema)?;
            }
            Ok(CollectionInfo {
                name: request.name.clone(),
                dimension: request.dimension.unwrap_or(0),
                metric: request.metric.clone(),
                count: 0,
                storage_mode: "graph".to_string(),
            })
        })
        .map_err(CommandError::from)?;

    emit_collection_created(&app, &request.name);
    Ok(result)
}

/// Adds an edge to the knowledge graph.
#[command]
pub async fn add_edge<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: AddEdgeRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let coll = require_graph_collection(&db, &request.collection)?;

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
            let coll = require_graph_collection(&db, &request.collection)?;

            let edges = if let Some(label) = &request.label {
                coll.get_edges(Some(label.as_str()))
            } else if let Some(source) = request.source {
                coll.get_outgoing(source)
            } else if let Some(target) = request.target {
                coll.get_incoming(target)
            } else {
                coll.get_edges(None)
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
            let coll = require_graph_collection(&db, &request.collection)?;

            let config = TraversalConfig::with_range(1, request.max_depth)
                .with_limit(request.limit)
                .with_rel_types(request.rel_types.unwrap_or_default());

            let results = if request.algorithm == "dfs" {
                coll.traverse_dfs(request.source, &config)
            } else {
                coll.traverse_bfs(request.source, &config)
            };

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
            let coll = require_graph_collection(&db, &request.collection)?;

            let (in_degree, out_degree) = coll.node_degree(request.node_id);

            Ok(NodeDegreeOutput {
                node_id: request.node_id,
                in_degree,
                out_degree,
            })
        })
        .map_err(CommandError::from)
}

/// Multi-source parallel BFS traversal with deduplication.
#[command]
pub async fn traverse_graph_parallel<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: TraverseGraphParallelRequest,
) -> std::result::Result<Vec<TraversalOutput>, CommandError> {
    state
        .with_db(|db| {
            let coll = require_graph_collection(&db, &request.collection)?;

            let config = TraversalConfig::with_range(1, request.max_depth)
                .with_limit(request.limit)
                .with_rel_types(request.rel_types.unwrap_or_default());

            let results = coll.traverse_bfs_parallel(&request.sources, &config);

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
