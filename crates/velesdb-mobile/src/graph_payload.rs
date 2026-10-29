//! Mapping between the mobile graph records and core's graph storage, and the
//! import of the JSON snapshots the former in-memory store wrote.
//!
//! Core keeps a graph node as a JSON payload whose `_labels` array holds its
//! labels: the key core's label index and VelesQL `MATCH (n:Label)` read
//! (`velesdb-core/src/collection/graph/label_index.rs`), whose first entry is
//! the node type. A [`MobileGraphNode`] carries one label beside its
//! properties, so it is stored as `{"_labels": [label], ...properties}`.
//! `velesql_match_sees_store_nodes` (`graph_tests.rs`) fails if the key or
//! its shape drifts from what core reads.

use std::collections::HashSet;

use serde_json::{Map, Value};
use velesdb_core::{GraphCollection, GraphEdge};

use crate::graph::{MobileGraphEdge, MobileGraphNode};
use crate::VelesError;

/// Payload key holding a core graph node's labels.
const LABELS_KEY: &str = "_labels";

/// Parses a record's `properties_json` into a JSON object; `owner` names the
/// record in the error.
fn parse_properties(json: Option<&str>, owner: &str) -> Result<Map<String, Value>, VelesError> {
    let Some(json) = json else {
        return Ok(Map::new());
    };
    match serde_json::from_str(json) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(VelesError::database(format!(
            "{owner}: properties_json must be a JSON object"
        ))),
        Err(e) => Err(VelesError::database(format!(
            "{owner}: invalid properties_json: {e}"
        ))),
    }
}

/// Builds the core payload of `node`: its properties plus `_labels`. An empty
/// label stores no `_labels` key, and reads back as an empty label.
pub(crate) fn node_payload(node: &MobileGraphNode) -> Result<Value, VelesError> {
    let owner = format!("node {}", node.id);
    let mut payload = parse_properties(node.properties_json.as_deref(), &owner)?;
    if payload.contains_key(LABELS_KEY) {
        return Err(VelesError::database(format!(
            "{owner}: properties_json must not carry the reserved `{LABELS_KEY}` key; \
             set `label` instead"
        )));
    }
    if !node.label.is_empty() {
        payload.insert(
            LABELS_KEY.to_string(),
            Value::from(vec![node.label.clone()]),
        );
    }
    Ok(Value::Object(payload))
}

/// Rebuilds a node from its core payload: the label is the first `_labels`
/// entry (core's node type), the other keys are the properties.
///
/// A payload that is not a JSON object (written through VelesQL, say) has no
/// labels to read; it is handed back whole as `properties_json`.
pub(crate) fn node_from_payload(
    id: u64,
    payload: Value,
    vector: Option<Vec<f32>>,
) -> MobileGraphNode {
    let Value::Object(mut properties) = payload else {
        return MobileGraphNode {
            id,
            label: String::new(),
            properties_json: Some(payload.to_string()),
            vector,
        };
    };
    let label = properties
        .remove(LABELS_KEY)
        .as_ref()
        .and_then(Value::as_array)
        .and_then(|labels| labels.first())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let properties_json = if properties.is_empty() {
        None
    } else {
        serde_json::to_string(&properties).ok()
    };
    MobileGraphNode {
        id,
        label,
        properties_json,
        vector,
    }
}

/// Whether a core node payload carries `label`, as `MATCH (n:label)` decides
/// it: any `_labels` entry matches. The empty label matches a node with none.
pub(crate) fn node_has_label(payload: &Value, label: &str) -> bool {
    let labels = payload.get(LABELS_KEY).and_then(Value::as_array);
    if label.is_empty() {
        return labels.is_none_or(Vec::is_empty);
    }
    labels.is_some_and(|labels| labels.iter().any(|l| l.as_str() == Some(label)))
}

/// Converts an edge record to core's edge. Core trims the label and rejects
/// an empty one.
pub(crate) fn edge_to_core(edge: &MobileGraphEdge) -> Result<GraphEdge, VelesError> {
    let owner = format!("edge {}", edge.id);
    let properties = parse_properties(edge.properties_json.as_deref(), &owner)?;
    Ok(
        GraphEdge::new(edge.id, edge.source, edge.target, &edge.label)?
            .with_properties(properties.into_iter().collect()),
    )
}

/// The JSON file the removed `MobileGraphStore.save` wrote.
#[derive(serde::Deserialize)]
struct LegacySnapshot {
    nodes: Vec<MobileGraphNode>,
    edges: Vec<MobileGraphEdge>,
}

/// Reads and parses the snapshot at `path`.
fn read_snapshot(path: &str) -> Result<LegacySnapshot, VelesError> {
    let bytes = std::fs::read(path).map_err(|e| {
        VelesError::database(format!("Graph snapshot read from '{path}' failed: {e}"))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|e| VelesError::database(format!("Graph snapshot '{path}' is not valid: {e}")))
}

/// Refuses a vector on a collection that stores no embeddings, before any
/// write, rather than after the nodes ahead of it were written.
fn check_embeddings(graph: &GraphCollection, nodes: &[MobileGraphNode]) -> Result<(), VelesError> {
    if graph.has_embeddings() {
        return Ok(());
    }
    match nodes.iter().find(|n| n.vector.is_some()) {
        Some(node) => Err(VelesError::database(format!(
            "node {} carries a vector, but this graph collection stores no embeddings; \
             create it with create_graph_collection_with_embeddings",
            node.id
        ))),
        None => Ok(()),
    }
}

/// Refuses an edge ID taken twice in the snapshot or already held by the
/// store, and an edge whose endpoint is neither in the snapshot nor stored.
fn check_edges(
    graph: &GraphCollection,
    snapshot_nodes: &HashSet<u64>,
    edges: &[GraphEdge],
) -> Result<(), VelesError> {
    let mut seen = HashSet::new();
    for edge in edges {
        if !seen.insert(edge.id()) || graph.has_edge(edge.id()) {
            return Err(VelesError::database(format!(
                "edge {} is already present; the snapshot import does not overwrite edges",
                edge.id()
            )));
        }
        for node in [edge.source(), edge.target()] {
            if !snapshot_nodes.contains(&node) && graph.get_node_payload(node)?.is_none() {
                return Err(VelesError::database(format!(
                    "edge {} references node {node}, which is neither in the snapshot nor \
                     in the store; add that node, then import again",
                    edge.id()
                )));
            }
        }
    }
    Ok(())
}

/// Writes the nodes: those without a vector under one durability barrier,
/// each embedded node through the vector upsert.
fn write_nodes(
    graph: &GraphCollection,
    nodes: Vec<(MobileGraphNode, Value)>,
) -> Result<(), VelesError> {
    let (embedded, plain): (Vec<_>, Vec<_>) =
        nodes.into_iter().partition(|(n, _)| n.vector.is_some());
    let entries: Vec<(u64, &Value)> = plain.iter().map(|(n, p)| (n.id, p)).collect();
    graph.upsert_node_payloads(&entries)?;
    for (node, payload) in embedded {
        graph.upsert_node(node.id, &payload, node.vector)?;
    }
    Ok(())
}

/// Loads the snapshot at `path` into `graph`.
///
/// Every refusal this binding can detect (malformed properties, an empty edge
/// label, a taken or repeated edge ID, a missing endpoint, a vector without
/// embeddings) happens before the first write. A storage failure, or core
/// refusing an embedding's dimension, can still stop the import part-way:
/// node writes are upserts, so importing the same file again after fixing
/// the cause completes it.
pub(crate) fn import_snapshot(graph: &GraphCollection, path: &str) -> Result<(), VelesError> {
    let snapshot = read_snapshot(path)?;
    check_embeddings(graph, &snapshot.nodes)?;
    let edges = snapshot
        .edges
        .iter()
        .map(edge_to_core)
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot_nodes: HashSet<u64> = snapshot.nodes.iter().map(|n| n.id).collect();
    check_edges(graph, &snapshot_nodes, &edges)?;
    let nodes = snapshot
        .nodes
        .into_iter()
        .map(|node| node_payload(&node).map(|payload| (node, payload)))
        .collect::<Result<Vec<_>, _>>()?;
    write_nodes(graph, nodes)?;
    graph.add_edges_batch(edges)?;
    Ok(())
}
