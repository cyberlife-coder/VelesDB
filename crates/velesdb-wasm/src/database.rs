//! `WasmDatabase` — named collection manager for in-browser use.
//!
//! Provides DDL-like lifecycle APIs (`create_collection`, `delete_collection`,
//! `list_collections`, `get_collection`) that mirror the REST server surface
//! in `velesdb-server`. All state is in-memory (no persistence feature).
//!
//! Inner functions return `Result<_, String>` for native-target testability;
//! the `#[wasm_bindgen]` methods convert to `JsValue` at the FFI boundary.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::graph_store::WasmGraphStore;
use crate::parsing;
use crate::store_new;
use crate::vector_store::VectorStore;

// ---------------------------------------------------------------------------
// Shared inner store
// ---------------------------------------------------------------------------

type SharedStore = Rc<RefCell<VectorStore>>;
type SharedGraph = Rc<RefCell<WasmGraphStore>>;

// ---------------------------------------------------------------------------
// Inner (testable) logic — returns String errors
// ---------------------------------------------------------------------------

/// Inner database state with `String`-error methods for native-target tests.
pub(crate) struct DatabaseInner {
    collections: HashMap<String, SharedStore>,
    /// In-memory graph stores keyed by collection name.
    ///
    /// Created lazily the first time any graph statement targets a given
    /// name. JS callers do not need to `CREATE COLLECTION` before running
    /// `INSERT EDGE` — the executor handles the auto-provision.
    graphs: HashMap<String, SharedGraph>,
}

impl DatabaseInner {
    pub(crate) fn new() -> Self {
        Self {
            collections: HashMap::new(),
            graphs: HashMap::new(),
        }
    }

    /// Returns a shared handle to the named graph store, creating it lazily
    /// when absent.
    pub(crate) fn graph_store(&mut self, name: &str) -> SharedGraph {
        if let Some(g) = self.graphs.get(name) {
            return Rc::clone(g);
        }
        let g = Rc::new(RefCell::new(WasmGraphStore::new()));
        self.graphs.insert(name.to_owned(), Rc::clone(&g));
        g
    }

    /// Returns a shared handle to the named graph store without creating
    /// one. Used by the executor when we don't want auto-provision (e.g.
    /// a DELETE or SELECT on a graph that must already exist).
    pub(crate) fn get_graph_store(&self, name: &str) -> Option<SharedGraph> {
        self.graphs.get(name).map(Rc::clone)
    }

    pub(crate) fn create_collection(
        &mut self,
        name: &str,
        dimension: usize,
        metric: &str,
    ) -> Result<(), String> {
        if self.collections.contains_key(name) {
            return Err(format!("Collection '{name}' already exists"));
        }
        let parsed_metric = parsing::parse_metric_inner(metric)?;
        let store = store_new::create_store(dimension, parsed_metric, crate::StorageMode::Full);
        self.collections
            .insert(name.to_owned(), Rc::new(RefCell::new(store)));
        Ok(())
    }

    /// Creates a metadata-only collection (dimension = 0, no vectors).
    ///
    /// Useful for storing reference data, lookups, or auxiliary information
    /// that does not require vector similarity search. Mirrors the Mobile
    /// bindings surface (`create_metadata_collection`).
    pub(crate) fn create_metadata_collection(&mut self, name: &str) -> Result<(), String> {
        if self.collections.contains_key(name) {
            return Err(format!("Collection '{name}' already exists"));
        }
        let store = store_new::create_metadata_only();
        self.collections
            .insert(name.to_owned(), Rc::new(RefCell::new(store)));
        Ok(())
    }

    pub(crate) fn delete_collection(&mut self, name: &str) -> Result<(), String> {
        if self.collections.remove(name).is_none() {
            return Err(format!("Collection '{name}' not found"));
        }
        Ok(())
    }

    pub(crate) fn collection_names(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }

    /// Returns `(name, dimension, is_metadata_only)` tuples for introspection.
    pub(crate) fn collection_summaries(&self) -> Vec<(String, usize, bool)> {
        self.collections
            .iter()
            .map(|(name, store)| {
                let borrowed = store.borrow();
                let dim = borrowed.dimension();
                (name.clone(), dim, dim == 0)
            })
            .collect()
    }

    pub(crate) fn get_shared_store(&self, name: &str) -> Result<SharedStore, String> {
        self.collections
            .get(name)
            .map(Rc::clone)
            .ok_or_else(|| format!("Collection '{name}' not found"))
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.collections.contains_key(name)
    }

    fn collection_count(&self) -> usize {
        self.collections.len()
    }
}

// ---------------------------------------------------------------------------
// WasmDatabase — wasm_bindgen facade
// ---------------------------------------------------------------------------

/// An in-memory database that manages named [`VectorStore`] collections.
///
/// # JavaScript usage
///
/// ```javascript
/// const db = new WasmDatabase();
/// db.create_collection("docs", 768, "cosine");
/// const coll = db.get_collection("docs");
/// coll.insert(1n, new Float32Array([...]));
/// const results = coll.search(new Float32Array([...]), 10);
/// db.delete_collection("docs");
/// ```
#[wasm_bindgen]
pub struct WasmDatabase {
    inner: DatabaseInner,
}

impl Default for WasmDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl WasmDatabase {
    /// Creates an empty database with no collections.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            inner: DatabaseInner::new(),
        }
    }

    /// Creates a named collection.
    ///
    /// # Arguments
    /// * `name` — unique collection name
    /// * `dimension` — vector dimensionality (e.g. 768)
    /// * `metric` — distance metric: `"cosine"`, `"euclidean"`, `"dot"`, etc.
    ///
    /// # Errors
    /// Returns an error if the collection already exists or the metric is
    /// invalid.
    pub fn create_collection(
        &mut self,
        name: &str,
        dimension: usize,
        metric: &str,
    ) -> Result<(), JsValue> {
        self.inner
            .create_collection(name, dimension, metric)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// Creates a metadata-only collection (no vectors, only payloads).
    ///
    /// A metadata-only collection accepts `INSERT`/`UPDATE`/`DELETE` via
    /// `execute_query()` without requiring a `vector` column, making it the
    /// ideal target for reference data, lookup tables, or auxiliary metadata.
    ///
    /// # Errors
    /// Returns an error if the collection already exists.
    #[wasm_bindgen(js_name = createMetadataCollection)]
    pub fn create_metadata_collection(&mut self, name: &str) -> Result<(), JsValue> {
        self.inner
            .create_metadata_collection(name)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// Deletes a named collection and frees its memory.
    ///
    /// **Note**: any [`WasmCollectionHandle`] previously obtained via
    /// [`get_collection`](Self::get_collection) remains functional after
    /// deletion (its `Rc` keeps the inner store alive). Creating a new
    /// collection with the same name will NOT reuse the old handle's data.
    ///
    /// # Errors
    /// Returns an error if the collection does not exist.
    pub fn delete_collection(&mut self, name: &str) -> Result<(), JsValue> {
        self.inner
            .delete_collection(name)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// Lists all collection names as a JavaScript `Array<string>`.
    pub fn list_collections(&self) -> JsValue {
        let names = self.inner.collection_names();
        serde_wasm_bindgen::to_value(&names).unwrap_or(JsValue::NULL)
    }

    /// Returns a mutable handle to an existing collection.
    ///
    /// The returned [`WasmCollectionHandle`] shares state with the database —
    /// inserts and deletions through the handle are visible in the database.
    ///
    /// # Errors
    /// Returns an error if the collection does not exist.
    pub fn get_collection(&self, name: &str) -> Result<WasmCollectionHandle, JsValue> {
        let store = self
            .inner
            .get_shared_store(name)
            .map_err(|e| JsValue::from_str(&e))?;
        Ok(WasmCollectionHandle { inner: store })
    }

    /// Returns the number of managed collections.
    #[wasm_bindgen(getter)]
    pub fn collection_count(&self) -> usize {
        self.inner.collection_count()
    }

    /// Executes a VelesQL statement against this database.
    ///
    /// Supports SELECT, INSERT / UPSERT, UPDATE, DELETE, DDL (CREATE / DROP /
    /// TRUNCATE COLLECTION), introspection (SHOW COLLECTIONS, DESCRIBE
    /// COLLECTION), and admin (FLUSH as no-op). Unsupported surfaces such as
    /// MATCH, TRAIN QUANTIZER, FUSION clauses, compound queries and graph
    /// DML return a descriptive error instead of crashing. See the
    /// [`velesql_exec`](crate::velesql_exec) module rustdoc for the full
    /// statement matrix.
    ///
    /// # Parameters
    /// * `sql` — VelesQL query string.
    /// * `params_json` — Optional JSON object with query parameters (keys
    ///   are bare names; use `$name` syntax in SQL). Pass `null` or `"{}"`
    ///   when no parameters are needed.
    ///
    /// # Example (JavaScript)
    /// ```javascript
    /// const db = new WasmDatabase();
    /// db.createMetadataCollection("docs");
    /// const r = db.executeQuery(
    ///     "INSERT INTO docs (id, title) VALUES (1, 'hello')",
    ///     null
    /// );
    /// console.log(r.kind, r.rowCount, r.rowsJson);
    /// ```
    ///
    /// # Errors
    /// Returns a `JsValue` error string when parsing fails, parameters are
    /// invalid, the target collection does not exist, or the statement uses
    /// a feature that WASM does not support.
    #[wasm_bindgen(js_name = executeQuery)]
    pub fn execute_query(
        &mut self,
        sql: &str,
        params_json: Option<String>,
    ) -> Result<crate::velesql_result::QueryResult, JsValue> {
        crate::velesql_exec::execute(&mut self.inner, sql, params_json.as_deref())
            .map_err(|e| JsValue::from_str(&e))
    }
}

// ---------------------------------------------------------------------------
// WasmCollectionHandle — wasm_bindgen facade for a single collection
// ---------------------------------------------------------------------------

/// A handle to a [`VectorStore`] managed by a [`WasmDatabase`].
///
/// Operations on this handle mutate the shared collection. Multiple handles
/// to the same collection are valid (single-threaded WASM — no data races).
#[wasm_bindgen]
pub struct WasmCollectionHandle {
    inner: SharedStore,
}

#[wasm_bindgen]
impl WasmCollectionHandle {
    /// Inserts a vector with the given ID.
    ///
    /// # Errors
    /// Returns an error if the vector dimension does not match the collection.
    pub fn insert(&self, id: u64, vector: &[f32]) -> Result<(), JsValue> {
        let mut store = self.inner.borrow_mut();
        store.insert(id, vector)
    }

    /// k-NN search. Returns `[[id, score], ...]`.
    ///
    /// # Errors
    /// Returns an error if the query dimension does not match the collection.
    pub fn search(&self, query: &[f32], k: usize) -> Result<JsValue, JsValue> {
        let store = self.inner.borrow();
        store.search(query, k)
    }

    /// Removes a vector by ID. Returns `true` if found.
    pub fn remove(&self, id: u64) -> bool {
        let mut store = self.inner.borrow_mut();
        store.remove(id)
    }

    /// Returns the number of vectors in the collection.
    #[wasm_bindgen(getter)]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Returns `true` if the collection has no vectors.
    #[wasm_bindgen(getter)]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Returns the vector dimensionality.
    #[wasm_bindgen(getter)]
    pub fn dimension(&self) -> usize {
        self.inner.borrow().dimension()
    }
}

// ---------------------------------------------------------------------------
// Tests (native target only — wasm32 uses wasm-bindgen-test)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "database_tests.rs"]
mod tests;
