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

use crate::parsing;
use crate::store_new;
use crate::vector_store::VectorStore;

// ---------------------------------------------------------------------------
// Shared inner store
// ---------------------------------------------------------------------------

type SharedStore = Rc<RefCell<VectorStore>>;

// ---------------------------------------------------------------------------
// Inner (testable) logic — returns String errors
// ---------------------------------------------------------------------------

/// Inner database state with `String`-error methods for native-target tests.
struct DatabaseInner {
    collections: HashMap<String, SharedStore>,
}

impl DatabaseInner {
    fn new() -> Self {
        Self {
            collections: HashMap::new(),
        }
    }

    fn create_collection(
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

    fn delete_collection(&mut self, name: &str) -> Result<(), String> {
        if self.collections.remove(name).is_none() {
            return Err(format!("Collection '{name}' not found"));
        }
        Ok(())
    }

    fn collection_names(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }

    fn get_shared_store(&self, name: &str) -> Result<SharedStore, String> {
        self.collections
            .get(name)
            .map(Rc::clone)
            .ok_or_else(|| format!("Collection '{name}' not found"))
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

    /// Deletes a named collection and frees its memory.
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
