//! Edge API of a WASM database: what REST `/collections/{name}/graph/edges`
//! does, over the graph store a MATCH on the same collection reads.
//!
//! VelesQL `INSERT EDGE` takes literal scalars only (no parameters, no nested
//! properties), so a client that builds edges from data would have to splice
//! strings into a statement. These methods take the edge as data instead.
//! As on the server, the collection must exist and an edge's endpoints must
//! be nodes of it (its points, or nodes written with `INSERT NODE`); reads go
//! through the same read gate as `SELECT EDGES`.

use serde::{Deserialize, Serialize};
use velesdb_core::api_types::serde_id;

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::graph_store::WasmEdge;
use crate::observer::WasmQueryOperationKind;

/// An edge as the client sends it (`GraphEdge` of the TS SDK and REST).
#[derive(Debug, Deserialize)]
pub(crate) struct EdgeInput {
    /// Explicit edge id, as a decimal string or a number; derived from
    /// (source, target, label) when absent.
    #[serde(
        default,
        deserialize_with = "serde_id::deserialize_option_id_from_string_or_number"
    )]
    pub id: Option<u64>,
    /// Source node id, as a decimal string or a number (as on REST).
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub source: u64,
    /// Target node id, as a decimal string or a number (as on REST).
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub target: u64,
    pub label: String,
    /// Edge properties: any JSON object.
    pub properties: Option<serde_json::Value>,
}

/// An edge as the client reads it, as REST's `EdgeResponse`: its id and its
/// ends are decimal strings. A derived edge id is hashed and routinely above
/// 2^53, and a node id may be, which `JSON.parse` would round.
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct EdgeOutput {
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub source: u64,
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub target: u64,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
}

impl From<&WasmEdge> for EdgeOutput {
    fn from(edge: &WasmEdge) -> Self {
        Self {
            id: edge.id,
            source: edge.source,
            target: edge.target,
            label: edge.label.clone(),
            properties: edge.payload.clone(),
        }
    }
}

/// An edge id as JavaScript passes it: the decimal string every edge read
/// answers (`getEdges`, `addEdge`, SELECT EDGES, MATCH `r.id`).
///
/// # Errors
///
/// Returns an error for anything but a decimal `u64`.
pub(crate) fn parse_edge_id(id: &str) -> Result<u64, ExecError> {
    // The JS surface's own input (an id as a string, for ids past 2^53).
    id.parse::<u64>().map_err(|_| {
        ExecError::refused(format!(
            "Invalid edge id '{id}': expected a decimal u64 string"
        ))
    })
}

impl DatabaseInner {
    /// Adds `edge` to `collection` and returns its id.
    ///
    /// # Errors
    ///
    /// Returns core's error when the collection does not exist
    /// (`CollectionNotFound`), an endpoint is not a node of it
    /// (`NodeNotFound`), or an explicit id is already taken (`EdgeExists`).
    pub(crate) fn add_edge(&mut self, collection: &str, edge: EdgeInput) -> Result<u64, ExecError> {
        self.require_collection(collection)?;
        for node in [edge.source, edge.target] {
            if !self.has_node(collection, node) {
                return Err(velesdb_core::Error::NodeNotFound(node).into());
            }
        }
        self.graph_store(collection).borrow_mut().insert_edge(
            edge.id,
            edge.source,
            edge.target,
            edge.label,
            edge.properties,
        )
    }

    /// The edges of `collection`, optionally only those labelled `label`.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection does not exist or the read gate
    /// denies it.
    pub(crate) fn edges(
        &self,
        collection: &str,
        label: Option<&str>,
    ) -> Result<Vec<EdgeOutput>, ExecError> {
        self.require_collection(collection)?;
        self.check_query_access(collection, WasmQueryOperationKind::GraphTraversal)?;
        Ok(self
            .get_graph_store(collection)
            .map_or_else(Vec::new, |store| {
                store
                    .borrow()
                    .filter_edges(None, None, label)
                    .map(EdgeOutput::from)
                    .collect()
            }))
    }

    /// Removes edge `id` from `collection`; `false` when it was not there.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection does not exist.
    pub(crate) fn remove_edge(&mut self, collection: &str, id: u64) -> Result<bool, ExecError> {
        self.require_collection(collection)?;
        Ok(self
            .get_graph_store(collection)
            .is_some_and(|store| store.borrow_mut().delete_edge_by_id(id)))
    }

    /// Removes the points `ids` of `collection` and the edges they are an
    /// end of, as core's `Collection::delete` cascades (#900); returns how
    /// many points were there. Points and edges live in separate stores, so
    /// a removal through the store handle alone would leave the edges behind
    /// and reconnect a later point under the same id.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection does not exist.
    pub(crate) fn remove_points(
        &mut self,
        collection: &str,
        ids: &[u64],
    ) -> Result<usize, ExecError> {
        let store = self.get_shared_store(collection)?;
        let removed = ids
            .iter()
            .filter(|&&id| store.borrow_mut().remove(id))
            .count();
        self.drop_edges_of(collection, ids);
        Ok(removed)
    }

    /// Drops every edge of `collection` that `ids` are an end of.
    pub(crate) fn drop_edges_of(&self, collection: &str, ids: &[u64]) {
        if let Some(graph) = self.get_graph_store(collection) {
            let ids: std::collections::HashSet<u64> = ids.iter().copied().collect();
            graph
                .borrow_mut()
                .delete_edges_where(|e| ids.contains(&e.source) || ids.contains(&e.target));
        }
    }

    /// The number of edges in `collection`.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection does not exist or the read gate
    /// denies it.
    pub(crate) fn edge_count(&self, collection: &str) -> Result<usize, ExecError> {
        self.require_collection(collection)?;
        self.check_query_access(collection, WasmQueryOperationKind::GraphTraversal)?;
        Ok(self
            .get_graph_store(collection)
            .map_or(0, |store| store.borrow().edges().len()))
    }

    /// Refuses a missing `collection` with core's `CollectionNotFound`.
    fn require_collection(&self, collection: &str) -> Result<(), velesdb_core::Error> {
        if self.contains(collection) {
            Ok(())
        } else {
            Err(velesdb_core::Error::CollectionNotFound(
                collection.to_string(),
            ))
        }
    }

    /// Whether `id` is a node of `collection`: one of its points, or a node
    /// written into its graph with `INSERT NODE`.
    fn has_node(&self, collection: &str, id: u64) -> bool {
        let is_point = self
            .get_shared_store(collection)
            .is_ok_and(|store| store.borrow().ids.contains(&id));
        is_point
            || self
                .get_graph_store(collection)
                .is_some_and(|graph| graph.borrow().get_node(id).is_some())
    }
}

#[cfg(test)]
#[path = "database_graph_tests.rs"]
mod tests;
