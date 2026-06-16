//! Knowledge Graph Tauri commands (EPIC-061/US-008 refactoring).
//!
//! Extracted from commands.rs to improve modularity.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::helpers::{parse_metric, require_graph_collection};
use crate::state::VelesDbState;
use crate::types::{
    AddEdgeRequest, AddEdgesBatchRequest, CollectionInfo, CreateGraphCollectionRequest, EdgeOutput,
    GetEdgesRequest, GetNodeDegreeRequest, NodeDegreeOutput, TraversalOutput,
    TraverseGraphParallelRequest, TraverseGraphRequest,
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
    _app: AppHandle<R>,
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

    Ok(result)
}

/// Builds a core [`GraphEdge`](velesdb_core::GraphEdge) from raw edge fields,
/// validating the edge and normalizing properties. Shared by [`add_edge`] and
/// [`add_edges_batch`].
fn build_edge(
    id: u64,
    source: u64,
    target: u64,
    label: &str,
    properties: Option<serde_json::Value>,
) -> std::result::Result<velesdb_core::GraphEdge, Error> {
    let properties: std::collections::HashMap<String, serde_json::Value> = match properties {
        Some(serde_json::Value::Object(map)) => map.into_iter().collect(),
        _ => std::collections::HashMap::new(),
    };
    velesdb_core::GraphEdge::new(id, source, target, label)
        .map_err(|e| Error::InvalidConfig(e.to_string()))
        .map(|edge| edge.with_properties(properties))
}

/// Builds a [`TraversalConfig`] from the raw request fields. Shared by
/// [`traverse_graph`] and [`traverse_graph_parallel`].
fn build_traversal_config(
    max_depth: u32,
    limit: usize,
    rel_types: Option<Vec<String>>,
) -> TraversalConfig {
    TraversalConfig::with_range(1, max_depth)
        .with_limit(limit)
        .with_rel_types(rel_types.unwrap_or_default())
}

/// Maps core traversal results into the API [`TraversalOutput`] shape. Shared by
/// [`traverse_graph`] and [`traverse_graph_parallel`].
fn to_traversal_outputs(
    results: Vec<velesdb_core::collection::graph::TraversalResult>,
) -> Vec<TraversalOutput> {
    results
        .into_iter()
        .map(|r| TraversalOutput {
            target_id: r.target_id,
            depth: r.depth,
            path: r.path,
        })
        .collect()
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
            let edge = build_edge(
                request.id,
                request.source,
                request.target,
                &request.label,
                request.properties,
            )?;
            coll.add_edge(edge)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Adds multiple edges to the knowledge graph in one batched operation.
///
/// Returns the number of edges inserted.
#[command]
pub async fn add_edges_batch<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: AddEdgesBatchRequest,
) -> std::result::Result<u64, CommandError> {
    state
        .with_db(|db| {
            let coll = require_graph_collection(&db, &request.collection)?;
            let edges = request
                .edges
                .into_iter()
                .map(|e| build_edge(e.id, e.source, e.target, &e.label, e.properties))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let added = coll
                .add_edges_batch(edges)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(u64::try_from(added).unwrap_or(u64::MAX))
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

            let config =
                build_traversal_config(request.max_depth, request.limit, request.rel_types);

            let results = if request.algorithm == "dfs" {
                coll.traverse_dfs(request.source, &config)
            } else {
                coll.traverse_bfs(request.source, &config)
            };

            Ok(to_traversal_outputs(results))
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

            let config =
                build_traversal_config(request.max_depth, request.limit, request.rel_types);

            let results = coll.traverse_bfs_parallel(&request.sources, &config);

            Ok(to_traversal_outputs(results))
        })
        .map_err(CommandError::from)
}

#[cfg(test)]
mod tests {
    use super::build_edge;

    #[test]
    fn test_build_edge_with_object_properties() {
        let edge = build_edge(1, 10, 20, "KNOWS", Some(serde_json::json!({"weight": 0.5})))
            .expect("valid edge");
        assert_eq!(edge.id(), 1);
        assert_eq!(edge.source(), 10);
        assert_eq!(edge.target(), 20);
        assert_eq!(edge.label(), "KNOWS");
    }

    #[test]
    fn test_build_edge_null_and_non_object_properties_default_empty() {
        // Null and non-object property payloads normalize to no properties
        // rather than erroring.
        assert!(build_edge(2, 1, 2, "L", None).is_ok());
        assert!(build_edge(3, 1, 2, "L", Some(serde_json::Value::Null)).is_ok());
        assert!(build_edge(4, 1, 2, "L", Some(serde_json::json!("scalar"))).is_ok());
    }
}
