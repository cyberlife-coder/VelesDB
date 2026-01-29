//! `IndexedDB` persistence for `GraphStore` (EPIC-053/US-003).
//!
//! Provides async save/load operations for graph data in browser applications.
//! Enables offline-first PWA with persistent knowledge graphs.

use crate::graph::{GraphEdge, GraphNode, GraphStore};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, IdbDatabase, IdbRequest, IdbTransactionMode};

const DB_NAME: &str = "velesdb_graphs";
const DB_VERSION: u32 = 1;
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

    /// Saves a graph to `IndexedDB` with the given name.
    #[wasm_bindgen]
    pub async fn save(&self, graph_name: &str, store: &GraphStore) -> Result<(), JsValue> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Database not initialized"))?;

        // Create read-write transaction
        let store_names = js_sys::Array::new();
        store_names.push(&JsValue::from_str(NODES_STORE));
        store_names.push(&JsValue::from_str(EDGES_STORE));
        store_names.push(&JsValue::from_str(META_STORE));

        let transaction =
            db.transaction_with_str_sequence_and_mode(&store_names, IdbTransactionMode::Readwrite)?;

        let nodes_store = transaction.object_store(NODES_STORE)?;
        let edges_store = transaction.object_store(EDGES_STORE)?;
        let meta_store = transaction.object_store(META_STORE)?;

        // Save all nodes using internal accessor
        let nodes = store.get_all_nodes_internal();
        for node in nodes {
            let key = format!("{graph_name}:{}", node.id());
            let value = serde_wasm_bindgen::to_value(&node)?;
            let request = nodes_store.put_with_key(&value, &JsValue::from_str(&key))?;
            wait_for_request(&request).await?;
        }

        // Save all edges using internal accessor
        let edges = store.get_all_edges_internal();
        for edge in edges {
            let key = format!("{graph_name}:{}", edge.id());
            let value = serde_wasm_bindgen::to_value(&edge)?;
            let request = edges_store.put_with_key(&value, &JsValue::from_str(&key))?;
            wait_for_request(&request).await?;
        }

        // Save metadata
        let now = js_sys::Date::now();
        let metadata = GraphMetadata {
            name: graph_name.to_string(),
            node_count: store.node_count(),
            edge_count: store.edge_count(),
            created_at: now,
            updated_at: now,
            version: 1,
        };
        let meta_value = serde_wasm_bindgen::to_value(&metadata)?;
        let meta_request = meta_store.put_with_key(&meta_value, &JsValue::from_str(graph_name))?;
        wait_for_request(&meta_request).await?;

        Ok(())
    }

    /// Loads a graph from `IndexedDB` by name.
    #[wasm_bindgen]
    pub async fn load(&self, graph_name: &str) -> Result<GraphStore, JsValue> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Database not initialized"))?;

        let store_names = js_sys::Array::new();
        store_names.push(&JsValue::from_str(NODES_STORE));
        store_names.push(&JsValue::from_str(EDGES_STORE));

        let transaction = db.transaction_with_str_sequence(&store_names)?;
        let nodes_store = transaction.object_store(NODES_STORE)?;
        let edges_store = transaction.object_store(EDGES_STORE)?;

        let mut graph = GraphStore::new();
        let _prefix = format!("{graph_name}:");

        // Load nodes
        let nodes_request = nodes_store.get_all()?;
        let nodes_result = wait_for_request(&nodes_request).await?;
        if !nodes_result.is_undefined() {
            let nodes_array: js_sys::Array = nodes_result.unchecked_into();
            for i in 0..nodes_array.length() {
                let node_js = nodes_array.get(i);
                if let Ok(node) = serde_wasm_bindgen::from_value::<GraphNode>(node_js) {
                    graph.add_node(node);
                }
            }
        }

        // Load edges
        let edges_request = edges_store.get_all()?;
        let edges_result = wait_for_request(&edges_request).await?;
        if !edges_result.is_undefined() {
            let edges_array: js_sys::Array = edges_result.unchecked_into();
            for i in 0..edges_array.length() {
                let edge_js = edges_array.get(i);
                if let Ok(edge) = serde_wasm_bindgen::from_value::<GraphEdge>(edge_js) {
                    let _ = graph.add_edge(edge);
                }
            }
        }

        Ok(graph)
    }

    /// Lists all saved graph names.
    #[wasm_bindgen]
    pub async fn list_graphs(&self) -> Result<js_sys::Array, JsValue> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Database not initialized"))?;

        let transaction = db.transaction_with_str(META_STORE)?;
        let store = transaction.object_store(META_STORE)?;

        let request = store.get_all_keys()?;
        let result = wait_for_request(&request).await?;

        Ok(result.unchecked_into())
    }

    /// Deletes a saved graph by name.
    #[wasm_bindgen]
    pub async fn delete_graph(&self, graph_name: &str) -> Result<(), JsValue> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Database not initialized"))?;

        let store_names = js_sys::Array::new();
        store_names.push(&JsValue::from_str(NODES_STORE));
        store_names.push(&JsValue::from_str(EDGES_STORE));
        store_names.push(&JsValue::from_str(META_STORE));

        let transaction =
            db.transaction_with_str_sequence_and_mode(&store_names, IdbTransactionMode::Readwrite)?;

        // Delete metadata
        let meta_store = transaction.object_store(META_STORE)?;
        let meta_request = meta_store.delete(&JsValue::from_str(graph_name))?;
        wait_for_request(&meta_request).await?;

        Ok(())
    }

    /// Gets metadata for a saved graph.
    #[wasm_bindgen]
    pub async fn get_metadata(&self, graph_name: &str) -> Result<JsValue, JsValue> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Database not initialized"))?;

        let transaction = db.transaction_with_str(META_STORE)?;
        let store = transaction.object_store(META_STORE)?;

        let request = store.get(&JsValue::from_str(graph_name))?;
        wait_for_request(&request).await
    }
}

/// Opens or creates the graph database.
async fn open_graph_db() -> Result<IdbDatabase, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window object"))?;
    let idb_factory = window
        .indexed_db()?
        .ok_or_else(|| JsValue::from_str("IndexedDB not available"))?;

    let request = idb_factory.open_with_u32(DB_NAME, DB_VERSION)?;

    // Set up upgrade handler to create object stores
    let request_clone = request.clone();
    let onupgradeneeded = Closure::once(move |_event: Event| {
        let db: IdbDatabase = request_clone
            .result()
            .expect("Failed to get result")
            .unchecked_into();

        let store_names = db.object_store_names();

        // Create nodes store
        if !contains_store(&store_names, NODES_STORE) {
            db.create_object_store(NODES_STORE)
                .expect("Failed to create nodes store");
        }

        // Create edges store
        if !contains_store(&store_names, EDGES_STORE) {
            db.create_object_store(EDGES_STORE)
                .expect("Failed to create edges store");
        }

        // Create metadata store
        if !contains_store(&store_names, META_STORE) {
            db.create_object_store(META_STORE)
                .expect("Failed to create metadata store");
        }
    });
    request.set_onupgradeneeded(Some(onupgradeneeded.as_ref().unchecked_ref()));
    onupgradeneeded.forget();

    let result = wait_for_request(&request).await?;
    Ok(result.unchecked_into())
}

/// Checks if a store name exists in the list.
fn contains_store(store_names: &web_sys::DomStringList, name: &str) -> bool {
    for i in 0..store_names.length() {
        if let Some(n) = store_names.get(i) {
            if n == name {
                return true;
            }
        }
    }
    false
}

/// Waits for an IDB request to complete.
async fn wait_for_request(request: &IdbRequest) -> Result<JsValue, JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let resolve_clone = resolve.clone();
        let onsuccess = Closure::once(move |_event: Event| {
            resolve_clone.call0(&JsValue::UNDEFINED).unwrap();
        });
        request.set_onsuccess(Some(onsuccess.as_ref().unchecked_ref()));
        onsuccess.forget();

        let onerror = Closure::once(move |_event: Event| {
            reject
                .call1(&JsValue::UNDEFINED, &JsValue::from_str("Request failed"))
                .unwrap();
        });
        request.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();
    });

    JsFuture::from(promise).await?;
    request.result()
}

#[cfg(test)]
mod tests {
    // Tests require wasm-bindgen-test and must be run with wasm-pack test --headless --chrome
    // See tests/graph_persistence_tests.rs
}
