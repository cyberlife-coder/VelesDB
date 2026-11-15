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

use crate::database::DatabaseInner;
use crate::graph_store::WasmEdge;
use crate::observer::WasmQueryOperationKind;

/// An edge as the client sends it (`GraphEdge` of the TS SDK and REST).
#[derive(Debug, Deserialize)]
pub(crate) struct EdgeInput {
    /// Explicit edge id; derived from (source, target, label) when absent.
    pub id: Option<u64>,
    pub source: u64,
    pub target: u64,
    pub label: String,
    /// Edge properties: any JSON object.
    pub properties: Option<serde_json::Value>,
}

/// An edge as the client reads it.
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct EdgeOutput {
    pub id: u64,
    pub source: u64,
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

impl DatabaseInner {
    /// Adds `edge` to `collection` and returns its id.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection does not exist, an endpoint is
    /// not a node of it, or an explicit id is already taken.
    pub(crate) fn add_edge(&mut self, collection: &str, edge: EdgeInput) -> Result<u64, String> {
        self.require_collection(collection)?;
        for node in [edge.source, edge.target] {
            if !self.has_node(collection, node) {
                return Err(format!(
                    "Node {node} not found in collection '{collection}'"
                ));
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
    ) -> Result<Vec<EdgeOutput>, String> {
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
    pub(crate) fn remove_edge(&mut self, collection: &str, id: u64) -> Result<bool, String> {
        self.require_collection(collection)?;
        Ok(self
            .get_graph_store(collection)
            .is_some_and(|store| store.borrow_mut().delete_edge_by_id(id)))
    }

    /// The number of edges in `collection`.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection does not exist or the read gate
    /// denies it.
    pub(crate) fn edge_count(&self, collection: &str) -> Result<usize, String> {
        self.require_collection(collection)?;
        self.check_query_access(collection, WasmQueryOperationKind::GraphTraversal)?;
        Ok(self
            .get_graph_store(collection)
            .map_or(0, |store| store.borrow().edges().len()))
    }

    fn require_collection(&self, collection: &str) -> Result<(), String> {
        if self.contains(collection) {
            Ok(())
        } else {
            Err(format!("Collection '{collection}' not found"))
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
