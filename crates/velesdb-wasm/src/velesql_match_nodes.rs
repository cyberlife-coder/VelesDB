//! The node set a MATCH walks: the graph store's nodes plus the points of the
//! collection that shares its name.
//!
//! Core keeps points and edges in one collection, and a node is a point: its
//! payload holds the node's properties and its `_labels` array the node's
//! labels. The WASM build keeps edges (and `INSERT NODE` payloads) in a graph
//! store beside the collection, so a MATCH must read both to see what core
//! sees. A node written with `INSERT NODE` wins over the point with the same
//! id: it is the more specific write, made against the graph.

use std::collections::HashMap;

use crate::graph_store::{WasmGraphNode, WasmGraphStore};
use crate::store_state::StoreState;

/// Payload key holding a node's labels, in core and in both WASM stores.
pub(crate) const LABELS_KEY: &str = "_labels";

/// The nodes one MATCH can bind, over borrowed stores.
pub(crate) struct MatchNodes<'a> {
    graph: &'a WasmGraphStore,
    /// Points of the same-named collection that the graph store does not
    /// already hold as nodes, keyed by id.
    points: HashMap<u64, WasmGraphNode>,
    /// The collection itself, for node vectors (`similarity()`).
    collection: Option<&'a StoreState>,
    /// Row index of every point id in `collection`.
    rows: HashMap<u64, usize>,
}

impl<'a> MatchNodes<'a> {
    /// Builds the node set from the graph store and, when one exists, the
    /// collection sharing its name.
    pub(crate) fn new(graph: &'a WasmGraphStore, collection: Option<&'a StoreState>) -> Self {
        let points = collection
            .map(|state| {
                state
                    .ids
                    .iter()
                    .zip(&state.payloads)
                    .filter(|(id, _)| graph.get_node(**id).is_none())
                    .map(|(&id, payload)| (id, point_node(payload.as_ref())))
                    .collect()
            })
            .unwrap_or_default();
        let rows = collection
            .map(|state| {
                state
                    .ids
                    .iter()
                    .enumerate()
                    .map(|(row, &id)| (id, row))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            graph,
            points,
            collection,
            rows,
        }
    }

    /// The vector of node `id`: the point's, when the collection holds one
    /// (a node written only with `INSERT NODE` has none). A metadata-only
    /// collection has no vectors.
    pub(crate) fn vector(&self, id: u64) -> Option<&[f32]> {
        let state = self.collection?;
        if state.dimension == 0 {
            return None;
        }
        let start = self.rows.get(&id)? * state.dimension;
        state.data.get(start..start + state.dimension)
    }

    /// The collection's dimension and metric, when it has vectors.
    pub(crate) fn vector_space(&self) -> Option<(usize, crate::DistanceMetric)> {
        self.collection
            .filter(|state| state.dimension > 0)
            .map(|state| (state.dimension, state.metric))
    }

    /// The graph store, whose edges the walk follows.
    pub(crate) fn graph(&self) -> &WasmGraphStore {
        self.graph
    }

    /// The node bound to `id`, if either store holds it.
    pub(crate) fn get(&self, id: u64) -> Option<&WasmGraphNode> {
        self.graph.get_node(id).or_else(|| self.points.get(&id))
    }

    /// Node ids a pattern may start from: every node carrying `label`, or
    /// every node when `label` is `None`. Point ids come after graph ids, in
    /// ascending order.
    pub(crate) fn candidates(&self, label: Option<&str>) -> Vec<u64> {
        let mut ids = self.graph.candidate_nodes(label);
        let mut point_ids: Vec<u64> = self
            .points
            .iter()
            .filter(|(_, node)| label.is_none_or(|l| node.labels.iter().any(|x| x == l)))
            .map(|(&id, _)| id)
            .collect();
        point_ids.sort_unstable();
        ids.extend(point_ids);
        ids
    }
}

/// A point seen as a graph node: its payload, and the strings of its
/// `_labels` array as labels.
fn point_node(payload: Option<&serde_json::Value>) -> WasmGraphNode {
    let labels = payload
        .and_then(|p| p.get(LABELS_KEY))
        .and_then(serde_json::Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    WasmGraphNode {
        payload: payload.cloned(),
        labels,
    }
}
