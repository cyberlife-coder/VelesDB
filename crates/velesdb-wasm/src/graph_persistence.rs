//! `IndexedDB` persistence for `GraphStore` (EPIC-053/US-003).
//!
//! Provides async save/load operations for graph data in browser applications.
//! Enables offline-first PWA with persistent knowledge graphs.

use crate::graph::{GraphEdge, GraphNode, GraphStore};
use crate::idb_helpers::{open_with_stores, wait_for_request, WriteTransaction};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use web_sys::{IdbDatabase, IdbObjectStore};

const DB_NAME: &str = "velesdb_graphs";
const NODES_STORE: &str = "nodes";
const EDGES_STORE: &str = "edges";
const META_STORE: &str = "metadata";

/// Metadata for a persisted graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMetadata {
    pub name: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub created_at: f64,
    pub updated_at: f64,
    pub version: u32,
}

/// `IndexedDB` persistence manager for `GraphStore`.
#[wasm_bindgen]
pub struct GraphPersistence {
    db: Option<IdbDatabase>,
}

impl Default for GraphPersistence {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl GraphPersistence {
    /// Creates a new `GraphPersistence` instance (call `init()` to open database).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        GraphPersistence { db: None }
    }

    /// Initializes the database connection. Must be called before save/load.
    #[wasm_bindgen]
    pub async fn init(&mut self) -> Result<(), JsValue> {
        let db = open_graph_db().await?;
        self.db = Some(db);
        Ok(())
    }

    /// Returns a reference to the open database, or an error if not initialized.
    fn require_db(&self) -> Result<&IdbDatabase, JsValue> {
        self.db
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Database not initialized"))
    }

    /// Saves a graph to `IndexedDB` with the given name, replacing what was
    /// saved under that name, and resolves once the save has committed.
    ///
    /// One transaction drops the graph's previous rows, writes the current
    /// ones and the metadata (keeping the first save's `created_at`): a node
    /// or edge removed since the last save does not come back on load, and a
    /// failed save leaves the previous one intact — every row is serialized
    /// before anything is written, and a failure once writing has begun
    /// aborts the transaction.
    #[wasm_bindgen]
    pub async fn save(&self, graph_name: &str, store: &GraphStore) -> Result<(), JsValue> {
        let db = self.require_db()?;

        // Serialized first: a value JS cannot hold (an id past 2^53) fails
        // here, before the previous rows are touched.
        let node_rows = store
            .get_all_nodes_internal()
            .into_iter()
            .map(|node| {
                Ok((
                    row_key(graph_name, node.id()),
                    crate::js_value::to_js_value(&node)?,
                ))
            })
            .collect::<Result<Vec<_>, JsValue>>()?;
        let edge_rows = store
            .get_all_edges_internal()
            .into_iter()
            .map(|edge| {
                Ok((
                    row_key(graph_name, edge.id()),
                    crate::js_value::to_js_value(&edge)?,
                ))
            })
            .collect::<Result<Vec<_>, JsValue>>()?;

        let transaction = WriteTransaction::begin(db, &all_store_names())?;
        let nodes_store = transaction.store(NODES_STORE)?;
        let edges_store = transaction.store(EDGES_STORE)?;
        let meta_store = transaction.store(META_STORE)?;

        delete_rows(&nodes_store, graph_name).await?;
        delete_rows(&edges_store, graph_name).await?;
        for (key, value) in &node_rows {
            wait_for_request(&nodes_store.put_with_key(value, &JsValue::from_str(key))?).await?;
        }
        for (key, value) in &edge_rows {
            wait_for_request(&edges_store.put_with_key(value, &JsValue::from_str(key))?).await?;
        }

        let now = js_sys::Date::now();
        let previous = wait_for_request(&meta_store.get(&JsValue::from_str(graph_name))?).await?;
        let created_at = serde_wasm_bindgen::from_value::<GraphMetadata>(previous)
            .map_or(now, |meta| meta.created_at);
        let metadata = GraphMetadata {
            name: graph_name.to_string(),
            node_count: store.node_count(),
            edge_count: store.edge_count(),
            created_at,
            updated_at: now,
            version: 1,
        };
        let meta_value = crate::js_value::to_js_value(&metadata)?;
        wait_for_request(&meta_store.put_with_key(&meta_value, &JsValue::from_str(graph_name))?)
            .await?;

        transaction.commit().await
    }

    /// Loads a graph from `IndexedDB` by name: exactly the rows saved under
    /// that name — never another graph's whose name merely starts with it.
    #[wasm_bindgen]
    pub async fn load(&self, graph_name: &str) -> Result<GraphStore, JsValue> {
        let db = self.require_db()?;

        let store_names = js_sys::Array::new();
        store_names.push(&JsValue::from_str(NODES_STORE));
        store_names.push(&JsValue::from_str(EDGES_STORE));

        let transaction = db.transaction_with_str_sequence(&store_names)?;
        let nodes_store = transaction.object_store(NODES_STORE)?;
        let edges_store = transaction.object_store(EDGES_STORE)?;

        let mut graph = GraphStore::new();
        for node_js in rows(&nodes_store, graph_name).await? {
            if let Ok(node) = serde_wasm_bindgen::from_value::<GraphNode>(node_js) {
                graph.add_node(node);
            }
        }
        for edge_js in rows(&edges_store, graph_name).await? {
            if let Ok(edge) = serde_wasm_bindgen::from_value::<GraphEdge>(edge_js) {
                let _ = graph.add_edge(edge);
            }
        }
        Ok(graph)
    }

    /// Lists all saved graph names.
    #[wasm_bindgen]
    pub async fn list_graphs(&self) -> Result<js_sys::Array, JsValue> {
        let db = self.require_db()?;

        let transaction = db.transaction_with_str(META_STORE)?;
        let store = transaction.object_store(META_STORE)?;

        let request = store.get_all_keys()?;
        let result = wait_for_request(&request).await?;

        Ok(result.unchecked_into())
    }

    /// Deletes a saved graph by name — its rows and metadata, and no other
    /// graph's — and resolves once the deletion has committed.
    #[wasm_bindgen]
    pub async fn delete_graph(&self, graph_name: &str) -> Result<(), JsValue> {
        let db = self.require_db()?;

        let transaction = WriteTransaction::begin(db, &all_store_names())?;
        delete_rows(&transaction.store(NODES_STORE)?, graph_name).await?;
        delete_rows(&transaction.store(EDGES_STORE)?, graph_name).await?;
        let meta_store = transaction.store(META_STORE)?;
        wait_for_request(&meta_store.delete(&JsValue::from_str(graph_name))?).await?;

        transaction.commit().await
    }

    /// Closes the database connection; `init()` opens it again. An open
    /// connection blocks a version upgrade and `deleteDatabase` from any tab.
    #[wasm_bindgen]
    pub fn close(&mut self) {
        if let Some(db) = self.db.take() {
            db.close();
        }
    }

    /// Gets metadata for a saved graph.
    #[wasm_bindgen]
    pub async fn get_metadata(&self, graph_name: &str) -> Result<JsValue, JsValue> {
        let db = self.require_db()?;

        let transaction = db.transaction_with_str(META_STORE)?;
        let store = transaction.object_store(META_STORE)?;

        let request = store.get(&JsValue::from_str(graph_name))?;
        wait_for_request(&request).await
    }
}

const GRAPH_STORES: &[&str] = &[NODES_STORE, EDGES_STORE, META_STORE];

impl Drop for GraphPersistence {
    fn drop(&mut self) {
        self.close();
    }
}

/// Opens or creates the graph database with its stores.
async fn open_graph_db() -> Result<IdbDatabase, JsValue> {
    open_with_stores(DB_NAME, GRAPH_STORES, "Failed to access graph DB result").await
}

/// Builds a JS array of all three object store names (nodes, edges, metadata).
fn all_store_names() -> js_sys::Array {
    let arr = js_sys::Array::new();
    arr.push(&JsValue::from_str(NODES_STORE));
    arr.push(&JsValue::from_str(EDGES_STORE));
    arr.push(&JsValue::from_str(META_STORE));
    arr
}

/// The key of a node or edge row: `{graph_name}:{id}`.
fn row_key(graph_name: &str, id: u64) -> String {
    format!("{graph_name}:{id}")
}

/// Whether `key` is a row of `graph_name`. The id is decimal, so the key is
/// read from the right: `a:b:2` is row 2 of graph `a:b`, not a row of `a`,
/// although it sorts inside `a`'s key range — the range alone made graph `a`
/// load, and delete, graph `a:b`'s rows.
fn is_row_of(key: &str, graph_name: &str) -> bool {
    key.strip_prefix(graph_name)
        .and_then(|rest| rest.strip_prefix(':'))
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Builds an `IDBKeyRange` holding every key that starts with `{graph_name}:`
/// — a superset of the graph's rows, narrowed by [`is_row_of`].
fn graph_key_range(graph_name: &str) -> Result<web_sys::IdbKeyRange, JsValue> {
    let lower = JsValue::from_str(&format!("{graph_name}:"));
    let upper = JsValue::from_str(&format!("{graph_name}:\u{ffff}"));
    web_sys::IdbKeyRange::bound(&lower, &upper)
}

/// The keys of `graph_name`'s rows in `store`.
async fn row_keys(store: &IdbObjectStore, graph_name: &str) -> Result<Vec<JsValue>, JsValue> {
    let range = graph_key_range(graph_name)?;
    let keys: js_sys::Array = wait_for_request(&store.get_all_keys_with_key(&range)?)
        .await?
        .unchecked_into();
    Ok(keys
        .iter()
        .filter(|key| key.as_string().is_some_and(|k| is_row_of(&k, graph_name)))
        .collect())
}

/// The values of `graph_name`'s rows in `store`. `getAllKeys` and `getAll`
/// over one range list records in the same key order, so the two zip.
async fn rows(store: &IdbObjectStore, graph_name: &str) -> Result<Vec<JsValue>, JsValue> {
    let range = graph_key_range(graph_name)?;
    let keys: js_sys::Array = wait_for_request(&store.get_all_keys_with_key(&range)?)
        .await?
        .unchecked_into();
    let values: js_sys::Array = wait_for_request(&store.get_all_with_key(&range)?)
        .await?
        .unchecked_into();
    Ok(keys
        .iter()
        .zip(values.iter())
        .filter(|(key, _)| key.as_string().is_some_and(|k| is_row_of(&k, graph_name)))
        .map(|(_, value)| value)
        .collect())
}

/// Deletes `graph_name`'s rows from `store`, and no other graph's.
async fn delete_rows(store: &IdbObjectStore, graph_name: &str) -> Result<(), JsValue> {
    for key in row_keys(store, graph_name).await? {
        wait_for_request(&store.delete(&key)?).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "graph_persistence_tests.rs"]
mod tests;
