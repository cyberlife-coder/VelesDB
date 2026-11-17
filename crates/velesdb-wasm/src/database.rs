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
use crate::observer::{gate, WasmObserver, WasmQueryOperationKind};
use crate::parsing;
use crate::store_new;
use crate::store_state::StoreState;
use crate::wasm_error::WasmError;
use crate::VectorStore;

// ---------------------------------------------------------------------------
// Shared inner store
// ---------------------------------------------------------------------------

pub(crate) type SharedStore = Rc<RefCell<StoreState>>;
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
    /// Optional read-path control-plane hook (audit F-5.4, #1392).
    ///
    /// `None` on the open path (zero overhead — the gate is a single `Option`
    /// check). When `Some`, every governed read (`search`, VelesQL `SELECT` /
    /// `MATCH` / `SELECT EDGES`) consults it before touching the store. Held
    /// as `Rc<dyn WasmObserver>` — deliberately non-`Send` to match the
    /// single-threaded `Rc<RefCell<VectorStore>>` store graph.
    observer: Option<Rc<dyn WasmObserver>>,
}

impl DatabaseInner {
    pub(crate) fn new() -> Self {
        Self {
            collections: HashMap::new(),
            graphs: HashMap::new(),
            observer: None,
        }
    }

    /// Registers (or replaces) the read-path observer.
    ///
    /// Reads routed through the database after this call consult the observer.
    /// Handles obtained via [`WasmDatabase::get_collection`] snapshot the
    /// observer at hand-out time, so register the observer **before** handing
    /// out handles you want governed (mirrors the existing DDL handle
    /// semantics documented on `WasmCollectionHandle`).
    pub(crate) fn register_observer(&mut self, observer: Rc<dyn WasmObserver>) {
        self.observer = Some(observer);
    }

    /// Returns a cloned handle to the current observer, if any. Used to
    /// snapshot the observer into a freshly minted `WasmCollectionHandle`.
    pub(crate) fn observer_handle(&self) -> Option<Rc<dyn WasmObserver>> {
        self.observer.clone()
    }

    /// Read-path gate for VelesQL / database-mediated reads.
    ///
    /// Zero-overhead when no observer is registered (single `Option` check).
    /// Returns `Err(msg)` when the observer denies the read.
    ///
    /// # Errors
    ///
    /// Propagates the observer's denial message.
    #[inline]
    pub(crate) fn check_query_access(
        &self,
        collection: &str,
        operation: WasmQueryOperationKind,
    ) -> Result<(), String> {
        gate(self.observer.as_ref(), collection, operation)
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

    /// Applies core's collection-name syntax rules (single source of truth).
    ///
    /// Without this, the lifecycle methods only ran a `HashMap` existence
    /// check, silently accepting names core would reject (path traversal,
    /// reserved device names, forbidden characters). Returns a structured
    /// [`WasmError`] carrying core's `VELES-034` code instead of pre-flattening
    /// to a bare string (backlog #22), so the FFI layer can surface
    /// `error.code` to browser clients.
    pub(crate) fn validate_name(name: &str) -> Result<(), WasmError> {
        velesdb_core::validate_collection_name(name).map_err(WasmError::from)
    }

    pub(crate) fn create_collection(
        &mut self,
        name: &str,
        dimension: usize,
        metric: &str,
    ) -> Result<(), String> {
        self.create_collection_with_mode(name, dimension, metric, crate::StorageMode::Full)
    }

    /// Creates a collection with an explicit storage mode (Full / SQ8 /
    /// Binary / PQ / RaBitQ): VelesQL `CREATE COLLECTION … WITH (storage =
    /// …)` runs it. `create_collection` delegates to this with
    /// `StorageMode::Full` for backward compatibility with the public WASM
    /// surface.
    pub(crate) fn create_collection_with_mode(
        &mut self,
        name: &str,
        dimension: usize,
        metric: &str,
        mode: crate::StorageMode,
    ) -> Result<(), String> {
        Self::validate_name(name).map_err(|e| e.message().to_owned())?;
        if self.collections.contains_key(name) {
            return Err(format!("Collection '{name}' already exists"));
        }
        let parsed_metric = parsing::parse_metric_inner(metric)?;
        let store = store_new::create_store(dimension, parsed_metric, mode);
        self.collections
            .insert(name.to_owned(), Rc::new(RefCell::new(store)));
        Ok(())
    }

    /// Registers `store` as collection `name` without copying it: whoever
    /// else holds the same `Rc` (a JS `VectorStore` handle) keeps reading
    /// and writing the very points VelesQL sees. Refuses a taken name, so an
    /// attach never silently replaces a collection.
    pub(crate) fn attach_shared_store(
        &mut self,
        name: &str,
        store: SharedStore,
    ) -> Result<(), String> {
        if self.collections.contains_key(name) {
            return Err(format!("Collection '{name}' already exists"));
        }
        self.collections.insert(name.to_owned(), store);
        Ok(())
    }

    /// Installs a pre-built store under `name`. Test-only helper used to
    /// seed non-Full storage modes without going through the public DDL
    /// surface (which currently only supports Full). Returns an error if
    /// a collection by that name already exists.
    #[cfg(test)]
    pub(crate) fn install_store(&mut self, name: &str, store: StoreState) -> Result<(), String> {
        self.attach_shared_store(name, Rc::new(RefCell::new(store)))
    }

    /// Creates a metadata-only collection (dimension = 0, no vectors).
    ///
    /// Useful for storing reference data, lookups, or auxiliary information
    /// that does not require vector similarity search. Mirrors the Mobile
    /// bindings surface (`create_metadata_collection`).
    pub(crate) fn create_metadata_collection(&mut self, name: &str) -> Result<(), String> {
        Self::validate_name(name).map_err(|e| e.message().to_owned())?;
        if self.collections.contains_key(name) {
            return Err(format!("Collection '{name}' already exists"));
        }
        let store = store_new::create_metadata_only();
        self.collections
            .insert(name.to_owned(), Rc::new(RefCell::new(store)));
        Ok(())
    }

    /// Drops the named collection and any graph store sharing that name.
    ///
    /// Collections and their associated graph stores share a single
    /// namespace: dropping a collection also drops its graph store so that
    /// a subsequent `CREATE COLLECTION` with the same name cannot surface
    /// ghost nodes/edges left over from the previous lifecycle. Fixes
    /// Devin review PR #594 finding #3.
    pub(crate) fn delete_collection(&mut self, name: &str) -> Result<(), String> {
        if self.collections.remove(name).is_none() {
            return Err(format!("Collection '{name}' not found"));
        }
        // Drop the associated graph store, if any. No error when absent:
        // many collections never have a graph store attached.
        self.graphs.remove(name);
        Ok(())
    }

    /// Clears every node and edge in the graph store keyed by `name`, if
    /// one exists. Used by `TRUNCATE COLLECTION` to keep the collection
    /// namespace intact while wiping its graph data.
    pub(crate) fn clear_graph_store(&mut self, name: &str) {
        if let Some(g) = self.graphs.get(name) {
            g.borrow_mut().clear();
        }
    }

    /// Returns `true` if a graph store is currently registered under
    /// `name`. Intended for tests that assert graph lifecycle semantics.
    #[cfg(test)]
    pub(crate) fn has_graph_store(&self, name: &str) -> bool {
        self.graphs.contains_key(name)
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

#[cfg(test)]
impl WasmDatabase {
    /// The executor state behind this handle, for native tests.
    pub(crate) fn inner_mut(&mut self) -> &mut DatabaseInner {
        &mut self.inner
    }
}

impl Default for WasmDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmDatabase {
    /// Registers a read-path observer (audit F-5.4, #1392).
    ///
    /// This is the internal Rust seam: `dyn WasmObserver` is not a
    /// `wasm_bindgen`-compatible type, so JS registration would require a
    /// concrete adapter wrapping a JS callback (documented follow-up). Reads
    /// routed through the database after this call — `search` on handles
    /// obtained afterwards, and every VelesQL `SELECT` / `MATCH` /
    /// `SELECT EDGES` — consult the observer before touching the store.
    pub fn register_observer(&mut self, observer: Rc<dyn WasmObserver>) {
        self.inner.register_observer(observer);
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
        // Validate the name through the structured path first so an invalid
        // name surfaces `error.code === "VELES-034"` to JS (backlog #22);
        // remaining failures (metric, already-exists) keep their string form.
        DatabaseInner::validate_name(name).map_err(WasmError::into_js_value)?;
        self.inner
            .create_collection(name, dimension, metric)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// Attaches an existing `VectorStore` as collection `name`, sharing its
    /// data rather than copying it: `executeQuery` then reads and writes the
    /// same points the store handle does, in both directions.
    ///
    /// # Errors
    /// Returns an error if `name` is invalid or already taken.
    #[wasm_bindgen(js_name = attachStore)]
    pub fn attach_store(&mut self, name: &str, store: &VectorStore) -> Result<(), JsValue> {
        DatabaseInner::validate_name(name).map_err(WasmError::into_js_value)?;
        self.inner
            .attach_shared_store(name, store.shared_state())
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
        DatabaseInner::validate_name(name).map_err(WasmError::into_js_value)?;
        self.inner
            .create_metadata_collection(name)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// Deletes a named collection and frees its memory.
    ///
    /// # Handle semantics
    ///
    /// Any [`WasmCollectionHandle`] previously obtained via
    /// [`get_collection`](Self::get_collection) keeps its own `Rc` to the
    /// backing store — mechanically the data is NOT freed on the spot.
    /// The handle continues to reference the *old* store, detached from
    /// the database. A subsequent `CREATE COLLECTION` with the same name
    /// allocates a **new** store, distinct from the old handle's data.
    ///
    /// Callers that want handles to observe later mutations should re-
    /// obtain a fresh handle after any CREATE. Note that `TRUNCATE`
    /// clears in place (see Devin Review Finding F12), so outstanding
    /// handles DO see the wipe — only DROP detaches.
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
        crate::js_value::to_js_value(&names).unwrap_or(JsValue::NULL)
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
        Ok(WasmCollectionHandle {
            inner: store,
            // Snapshot the observer registered at hand-out time so the handle
            // can gate its own reads without a back-reference to the database.
            observer: self.inner.observer_handle(),
            name: name.to_owned(),
        })
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
    /// `velesql_exec` module rustdoc for the full
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

    /// Runs a VelesQL `MATCH` and returns its rows in core's `MatchResult`
    /// shape, as a JSON array string: `[{ bindings, depth, projected, score? }]`
    /// — the rows of REST `POST /collections/{name}/match`.
    ///
    /// `executeQuery` runs the same MATCH (same read gate, walk, ORDER BY and
    /// LIMIT) but returns alias-keyed objects and leaves RETURN to the
    /// caller. Name the collection with `_collection` in `params_json` (MATCH
    /// has no `FROM`); its points are the nodes, so `similarity()` scores
    /// their vectors. `vector` and `threshold` are REST `/match`'s: each row's
    /// last node is scored, rows past the threshold (default `0.0`) are
    /// dropped and the rest ordered by score. A row carries `score` when the
    /// MATCH computes one.
    ///
    /// # Example (JavaScript)
    /// ```javascript
    /// const rows = JSON.parse(db.executeMatch(
    ///     "MATCH (a:Doc)-[:CITES]->(b:Doc) WHERE similarity(b.embedding, $v) > 0.8 RETURN b.title",
    ///     JSON.stringify({ _collection: "docs", v: [1, 0] })
    /// ));
    /// // [{ bindings: { a: 1, b: 2 }, depth: 1, projected: { "b.title": "…" }, score: 0.97 }]
    /// ```
    ///
    /// # Errors
    /// Returns a `JsValue` error string when parsing or validation fails, the
    /// statement is not a MATCH, parameters are invalid, the read gate denies
    /// a collection, `vector` has another dimension than the collection,
    /// `threshold` is outside `[0, 1]`, or the MATCH fails.
    #[wasm_bindgen(js_name = executeMatch)]
    pub fn execute_match(
        &mut self,
        sql: &str,
        params_json: Option<String>,
        vector: Option<Vec<f32>>,
        threshold: Option<f32>,
    ) -> Result<String, JsValue> {
        let scoring =
            vector.map(|vector| crate::velesql_match_core::MatchScoring { vector, threshold });
        let rows = crate::velesql_match_core::execute_match_rows(
            &mut self.inner,
            sql,
            params_json.as_deref(),
            scoring,
        )
        .map_err(|e| JsValue::from_str(&e))?;
        serde_json::to_string(&rows).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Adds an edge to collection `name` and returns its id (a decimal
    /// string, as `getEdges` lists it), as REST
    /// `POST /collections/{name}/graph/edges` does. `edge_json` is a JSON
    /// object `{ id?, source, target, label, properties? }` (`id` a decimal
    /// string or a number); without `id` the
    /// id is derived from (source, target, label) as on every VelesDB engine.
    /// A MATCH on `name` reads the edge.
    ///
    /// # Errors
    /// Returns a `JsValue` error string when `edge_json` is malformed, the
    /// collection does not exist, an endpoint is not one of its nodes (its
    /// points, or nodes written with `INSERT NODE`), or the id is taken.
    #[wasm_bindgen(js_name = addEdge)]
    pub fn add_edge(&mut self, name: &str, edge_json: &str) -> Result<String, JsValue> {
        let edge = serde_json::from_str(edge_json)
            .map_err(|e| JsValue::from_str(&format!("Invalid edge: {e}")))?;
        self.inner
            .add_edge(name, edge)
            .map(|id| id.to_string())
            .map_err(|e| JsValue::from_str(&e))
    }

    /// The edges of collection `name` as a JSON array string of
    /// `{ id, source, target, label, properties? }`, optionally only those
    /// labelled `label`. `id` is a decimal string (derived edge ids exceed
    /// 2^53); `source` and `target` are numbers.
    ///
    /// # Errors
    /// Returns a `JsValue` error string when the collection does not exist or
    /// the read gate denies it.
    #[wasm_bindgen(js_name = getEdges)]
    pub fn get_edges(&self, name: &str, label: Option<String>) -> Result<String, JsValue> {
        let edges = self
            .inner
            .edges(name, label.as_deref())
            .map_err(|e| JsValue::from_str(&e))?;
        serde_json::to_string(&edges).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Removes edge `id` (its decimal string, as `getEdges` lists it) from
    /// collection `name`; `false` when it was absent.
    ///
    /// # Errors
    /// Returns a `JsValue` error string when `id` is not a decimal `u64` or
    /// the collection does not exist.
    #[wasm_bindgen(js_name = removeEdge)]
    pub fn remove_edge(&mut self, name: &str, id: &str) -> Result<bool, JsValue> {
        let id = crate::database_graph::parse_edge_id(id).map_err(|e| JsValue::from_str(&e))?;
        self.inner
            .remove_edge(name, id)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// The number of edges in collection `name`.
    ///
    /// # Errors
    /// Returns a `JsValue` error string when the collection does not exist or
    /// the read gate denies it.
    #[wasm_bindgen(js_name = edgeCount)]
    pub fn edge_count(&self, name: &str) -> Result<usize, JsValue> {
        self.inner
            .edge_count(name)
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
///
/// # Lifetime across DDL (Devin Review Finding F12)
///
/// | Operation  | Effect on outstanding handles                              |
/// |------------|-----------------------------------------------------------|
/// | `TRUNCATE` | Handle keeps pointing to the same (now-emptied) store.    |
/// | `DROP`     | Handle retains its `Rc` to the OLD store, detached from    |
/// |            | the database. A later `CREATE` with the same name creates |
/// |            | a NEW store that the old handle does not see.             |
/// | `CREATE`   | Fresh handles obtained via `get_collection` are required  |
/// |            | to observe the new store when reusing a dropped name.     |
#[wasm_bindgen]
pub struct WasmCollectionHandle {
    inner: SharedStore,
    /// Observer snapshotted from the database at [`WasmDatabase::get_collection`]
    /// time. `None` (the common case) makes [`Self::search`] gate a no-op.
    observer: Option<Rc<dyn WasmObserver>>,
    /// Collection name, carried for the read-path gate context.
    name: String,
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

    /// Bulk insert from a flat row-major `vectors` buffer plus parallel `ids`
    /// (VRB1 raw-bulk contract). No payloads.
    ///
    /// # Errors
    /// Returns an error on dimension mismatch or when
    /// `vectors.len() != ids.len() * dimension`.
    #[wasm_bindgen(js_name = insertBatchRaw)]
    pub fn insert_batch_raw(
        &self,
        ids: &[u64],
        vectors: &[f32],
        dimension: usize,
    ) -> Result<(), JsValue> {
        self.inner
            .borrow_mut()
            .insert_batch_raw(ids, vectors, dimension)
    }

    /// k-NN search. Returns `[[id, score], ...]`.
    ///
    /// When a read-path observer is registered on the owning database, it is
    /// consulted before the store is touched: a `Deny` decision aborts with
    /// the observer's message and produces no results (audit F-5.4, #1392).
    ///
    /// # Errors
    /// Returns an error if the query dimension does not match the collection,
    /// or if the read-path observer denies the query.
    pub fn search(&self, query: &[f32], k: usize) -> Result<JsValue, JsValue> {
        gate(
            self.observer.as_ref(),
            &self.name,
            WasmQueryOperationKind::VectorSearch,
        )
        .map_err(|e| JsValue::from_str(&e))?;
        let store = self.inner.borrow();
        store.search(query, k)
    }

    /// Removes a vector by ID. Returns `true` if found.
    pub fn remove(&self, id: u64) -> bool {
        let mut store = self.inner.borrow_mut();
        store.remove(id)
    }

    /// Returns the number of vectors in the collection.
    ///
    /// The count is a collection read: a registered observer's `Deny` aborts
    /// the getter with its message, as on `search`.
    ///
    /// # Errors
    /// Returns the observer's denial message.
    #[wasm_bindgen(getter)]
    pub fn len(&self) -> Result<usize, JsValue> {
        gate(
            self.observer.as_ref(),
            &self.name,
            WasmQueryOperationKind::Select,
        )
        .map_err(|e| JsValue::from_str(&e))?;
        Ok(self.inner.borrow().len())
    }

    /// Returns `true` if the collection has no vectors.
    ///
    /// Emptiness derives from the count — gated like [`Self::len`].
    ///
    /// # Errors
    /// Returns the observer's denial message.
    #[wasm_bindgen(getter)]
    pub fn is_empty(&self) -> Result<bool, JsValue> {
        Ok(self.len()? == 0)
    }

    /// Returns the vector dimensionality.
    #[wasm_bindgen(getter)]
    pub fn dimension(&self) -> usize {
        self.inner.borrow().dimension()
    }
}

impl WasmCollectionHandle {
    /// Test-only: whether this handle snapshotted a read-path observer at
    /// hand-out time. Used to assert the [`WasmDatabase::get_collection`]
    /// observer-snapshot wiring without touching the `JsValue`-returning
    /// `search` (which panics off `wasm32`).
    #[cfg(test)]
    pub(crate) fn has_observer(&self) -> bool {
        self.observer.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests (native target only — wasm32 uses wasm-bindgen-test)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "database_tests.rs"]
mod tests;
