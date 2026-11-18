//! The JS-facing `VectorStore`: a handle over a shared [`StoreState`].
//!
//! `VectorStore` used to own its data, so a store created from JavaScript
//! could never be seen by a [`crate::WasmDatabase`]: the database's VelesQL
//! executor ran over its own collections, a second copy at best. The data now
//! lives in an `Rc<RefCell<StoreState>>` that this handle and a database share
//! (see [`crate::WasmDatabase::attach_store`]), so `executeQuery` reads and
//! writes exactly the points the handle does.
//!
//! Every method delegates to [`StoreState`] with an identical signature, so
//! the generated JavaScript API is unchanged. A borrow lasts one call and is
//! never held across an `await`.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::database::SharedStore;
use crate::store_state::StoreState;
#[cfg(doc)]
use crate::{sparse, StorageMode};

/// A vector store for in-memory vector search.
///
/// # Performance
///
/// Uses contiguous memory layout for optimal cache locality and fast
/// serialization. Vector data is stored in a single buffer rather than
/// individual Vec allocations.
///
/// # Storage Modes
///
/// Every mode keeps the f32 vectors and scores them exactly, returning the
/// same results as a `velesdb-core` collection with that mode; the mode saves
/// no memory in the browser (see [`StorageMode`]).
///
/// A handle over shared state; see the module docs.
#[wasm_bindgen]
pub struct VectorStore {
    state: SharedStore,
}

impl VectorStore {
    fn from_state(state: StoreState) -> Self {
        Self {
            state: Rc::new(RefCell::new(state)),
        }
    }

    /// The shared state, for a database to attach without copying.
    pub(crate) fn shared_state(&self) -> SharedStore {
        Rc::clone(&self.state)
    }
}

#[wasm_bindgen]
impl VectorStore {
    /// Creates a new vector store. Metrics: cosine, euclidean, dot, hamming, jaccard.
    #[wasm_bindgen(constructor)]
    pub fn new(dimension: usize, metric: &str) -> Result<VectorStore, JsValue> {
        StoreState::new(dimension, metric).map(Self::from_state)
    }

    /// Checks a search-quality preset (`fast`, `balanced`, `accurate`,
    /// `perfect`, `autotune`, `custom:<ef>`, `adaptive:<min>:<max>`) with
    /// the parser `search_with_quality` runs, without a store or a search:
    /// what a caller validating a preset ahead of a search asks. Throws the
    /// parser's refusal; returns nothing otherwise.
    #[wasm_bindgen]
    pub fn validate_search_quality(quality: &str) -> Result<(), JsValue> {
        crate::parsing::parse_search_quality(quality)
    }

    /// Creates a metadata-only store (no vectors, only payloads).
    #[wasm_bindgen]
    pub fn new_metadata_only() -> VectorStore {
        Self::from_state(StoreState::new_metadata_only())
    }

    /// Returns true if this is a metadata-only store.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.state.borrow().is_metadata_only()
    }

    /// Creates store with a storage mode: full, sq8, binary, pq or rabitq.
    ///
    /// Every mode keeps f32 vectors (4 bytes per dimension) and returns the
    /// same results as `velesdb-core` with that mode.
    #[wasm_bindgen]
    pub fn new_with_mode(
        dimension: usize,
        metric: &str,
        mode: &str,
    ) -> Result<VectorStore, JsValue> {
        StoreState::new_with_mode(dimension, metric, mode).map(Self::from_state)
    }

    /// Returns the storage mode.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn storage_mode(&self) -> String {
        self.state.borrow().storage_mode()
    }

    /// Returns the number of vectors in the store.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.borrow().len()
    }

    /// Returns true if the store is empty.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.borrow().is_empty()
    }

    /// Returns the vector dimension.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.state.borrow().dimension()
    }

    /// Inserts a vector with the given ID.
    #[wasm_bindgen]
    pub fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), JsValue> {
        self.state.borrow_mut().insert(id, vector)
    }

    /// Inserts a vector with ID and optional JSON payload.
    #[wasm_bindgen]
    pub fn insert_with_payload(
        &mut self,
        id: u64,
        vector: &[f32],
        payload: JsValue,
    ) -> Result<(), JsValue> {
        self.state
            .borrow_mut()
            .insert_with_payload(id, vector, payload)
    }

    /// Gets a vector by ID. Returns `{id, vector, payload}` (`id` a decimal
    /// string) or null.
    #[wasm_bindgen]
    pub fn get(&self, id: u64) -> Result<JsValue, JsValue> {
        self.state.borrow().get(id)
    }

    /// Searches with metadata filtering. Returns `[{id, score, payload}]`,
    /// `id` the point's exact decimal string.
    #[wasm_bindgen]
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: JsValue,
    ) -> Result<JsValue, JsValue> {
        self.state.borrow().search_with_filter(query, k, filter)
    }

    /// k-NN search. Returns `[[id, score], ...]`, each id the point's exact
    /// decimal string (as REST sends it), each score a number.
    #[wasm_bindgen]
    pub fn search(&self, query: &[f32], k: usize) -> Result<JsValue, JsValue> {
        self.state.borrow().search(query, k)
    }

    /// k-NN search with a named quality mode (API parity with Server/Python/Mobile).
    ///
    /// Accepts the same quality strings as the Python and Server SDKs:
    /// `"fast"`, `"balanced"`, `"accurate"`, `"perfect"`, `"autotune"`,
    /// `"custom:<ef>"`, `"adaptive:<min_ef>:<max_ef>"`.
    ///
    /// **WASM note**: The current WASM VectorStore uses brute-force O(n)
    /// search (no HNSW graph), so all quality modes produce identical results.
    /// The quality parameter is parsed and validated for API parity and
    /// forward-compatibility with future HNSW-backed WASM stores.
    ///
    /// Returns `[[id, score], ...]` sorted by relevance, each id a decimal
    /// string.
    #[wasm_bindgen]
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: &str,
    ) -> Result<JsValue, JsValue> {
        self.state.borrow().search_with_quality(query, k, quality)
    }

    /// Similarity search with threshold. Operators: >, >=, <, <=, =, !=.
    ///
    /// Returns `[[id, score], ...]`, each id a decimal string.
    #[wasm_bindgen]
    pub fn similarity_search(
        &self,
        query: &[f32],
        threshold: f32,
        operator: &str,
        k: usize,
    ) -> Result<JsValue, JsValue> {
        self.state
            .borrow()
            .similarity_search(query, threshold, operator, k)
    }

    /// Text search on payload fields (substring matching).
    ///
    /// Returns `[{id, score, payload}, ...]` in insertion order, truncated to
    /// `k`, with `score: null`: a substring match selects, it does not rank.
    /// `id` is a decimal string.
    #[wasm_bindgen]
    pub fn text_search(
        &self,
        query: &str,
        k: usize,
        field: Option<String>,
    ) -> Result<JsValue, JsValue> {
        self.state.borrow().text_search(query, k, field)
    }

    /// Hybrid search (vector + text). `vector_weight` 0-1 (default 0.5); a
    /// non-finite weight is refused (`VELES-009`), as core refuses it.
    ///
    /// Computes the exact per-vector score and the text match in a single
    /// pass, in every storage mode.
    ///
    /// Returns `[{id, score, payload}, ...]` sorted by combined score
    /// descending, truncated to `k` results. `id` is a decimal string.
    #[wasm_bindgen]
    pub fn hybrid_search(
        &self,
        query_vector: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        self.state
            .borrow()
            .hybrid_search(query_vector, text_query, k, vector_weight)
    }

    /// Sparse k-NN search over the store's pre-built sparse index.
    ///
    /// Unlike [`sparse_search`](Self::sparse_search) — which returns an empty
    /// result set when no sparse data has been inserted — this method mirrors
    /// the core `sparse_search` contract and returns an **error** when the
    /// store has no sparse index, so callers can distinguish "index missing"
    /// from "no matches". Scoring is delegated unchanged to the
    /// recall-verified [`sparse::SparseIndex`] kernel.
    ///
    /// Returns a JSON array of `{doc_id, score}` objects sorted by score
    /// descending, truncated to `k`. `doc_id` is a decimal string.
    #[wasm_bindgen]
    pub fn search_sparse(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<JsValue, JsValue> {
        self.state
            .borrow()
            .search_sparse(query_indices, query_values, k)
    }

    /// VelesQL-style query returning multi-model results (EPIC-031 US-009).
    ///
    /// Returns results in `HybridResult` format with `node_id`, `vector_score`,
    /// `graph_score`, `fused_score`, `bindings`, and `column_data`.
    ///
    /// # Arguments
    /// * `query_vector` - Query vector for similarity search
    /// * `k` - Number of results to return
    ///
    /// # Returns
    /// Array of `{nodeId, vectorScore, graphScore, fusedScore, bindings, columnData}`,
    /// `nodeId` a decimal string.
    #[wasm_bindgen]
    pub fn query(&self, query_vector: &[f32], k: usize) -> Result<JsValue, JsValue> {
        self.state.borrow().query(query_vector, k)
    }

    /// Multi-query search with fusion. Strategies: average, maximum, weighted, rrf, relative_score/rsf.
    ///
    /// `weights` is an optional `[avg_weight, max_weight, hit_weight]` triple
    /// used only by the `"weighted"` strategy. When omitted, core's canonical
    /// default weights are used (`avg=0.6, max=0.3, hit=0.1`) — see issue
    /// #1545. Passing a slice of any length other than 3 is an error.
    ///
    /// Returns `[[id, score], ...]`, each id a decimal string.
    #[wasm_bindgen]
    #[allow(clippy::too_many_arguments)]
    pub fn multi_query_search(
        &mut self,
        vectors: &[f32],
        num_vectors: usize,
        k: usize,
        strategy: &str,
        rrf_k: Option<u32>,
        weights: Option<Vec<f32>>,
    ) -> Result<JsValue, JsValue> {
        self.state.borrow_mut().multi_query_search(
            vectors,
            num_vectors,
            k,
            strategy,
            rrf_k,
            weights,
        )
    }

    /// Batch search for multiple vectors. Returns `[[[id, score], ...], ...]`,
    /// each id a decimal string.
    #[wasm_bindgen]
    pub fn batch_search(
        &self,
        vectors: &[f32],
        num_vectors: usize,
        k: usize,
    ) -> Result<JsValue, JsValue> {
        self.state.borrow().batch_search(vectors, num_vectors, k)
    }

    /// Removes a vector by ID.
    #[wasm_bindgen]
    pub fn remove(&mut self, id: u64) -> bool {
        self.state.borrow_mut().remove(id)
    }

    /// Clears all vectors, payloads, and auxiliary indexes from the
    /// store in place.
    ///
    /// Preserves `dimension`, `metric`, and `storage_mode` so the store
    /// can be re-populated without reallocation. Any sparse index is
    /// dropped: it would otherwise reference rows that no longer exist.
    ///
    /// Used by `TRUNCATE` (Devin Review Finding F12) so that outstanding
    /// `WasmCollectionHandle`s holding an `Rc` to this store observe the
    /// wipe atomically instead of pointing to a stale clone.
    #[wasm_bindgen]
    pub fn clear(&mut self) {
        self.state.borrow_mut().clear()
    }

    /// Returns memory usage estimate in bytes: ids plus the f32 vectors,
    /// which every storage mode keeps (see [`StorageMode`]).
    #[wasm_bindgen]
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.state.borrow().memory_usage()
    }

    /// Creates store with pre-allocated capacity.
    #[wasm_bindgen]
    pub fn with_capacity(
        dimension: usize,
        metric: &str,
        capacity: usize,
    ) -> Result<VectorStore, JsValue> {
        StoreState::with_capacity(dimension, metric, capacity).map(Self::from_state)
    }

    /// Pre-allocates memory for additional vectors.
    #[wasm_bindgen]
    pub fn reserve(&mut self, additional: usize) {
        self.state.borrow_mut().reserve(additional)
    }

    /// Batch insert. Input: `[[id, Float32Array], ...]`.
    #[wasm_bindgen]
    pub fn insert_batch(&mut self, batch: JsValue) -> Result<(), JsValue> {
        self.state.borrow_mut().insert_batch(batch)
    }

    /// Bulk insert from a flat contiguous `Float32Array` (zero-copy layout),
    /// mirroring the VRB1 raw-bulk contract: `ids` + flat row-major `vectors`
    /// + `dimension`. No payloads (use `insert_batch` for those).
    #[wasm_bindgen(js_name = insertBatchRaw)]
    pub fn insert_batch_raw(
        &mut self,
        ids: &[u64],
        vectors: &[f32],
        dimension: usize,
    ) -> Result<(), JsValue> {
        self.state
            .borrow_mut()
            .insert_batch_raw(ids, vectors, dimension)
    }

    /// Exports to binary format for IndexedDB/localStorage: ids, dense
    /// vectors, payloads, configuration and the sparse index (format v4).
    #[wasm_bindgen]
    pub fn export_to_bytes(&self) -> Result<Vec<u8>, JsValue> {
        self.state.borrow().export_to_bytes()
    }

    /// Saves to `IndexedDB`, as [`Self::export_to_bytes`] serializes the
    /// store, sparse index included.
    #[wasm_bindgen]
    pub async fn save(&self, db_name: &str) -> Result<(), JsValue> {
        let bytes = self.state.borrow().export_to_bytes()?;
        crate::persistence::save_to_indexeddb(db_name, &bytes).await
    }

    /// Loads from `IndexedDB`.
    #[wasm_bindgen]
    pub async fn load(db_name: &str) -> Result<VectorStore, JsValue> {
        StoreState::load(db_name).await.map(Self::from_state)
    }

    /// Deletes `IndexedDB` database.
    #[wasm_bindgen]
    pub async fn delete_database(db_name: &str) -> Result<(), JsValue> {
        StoreState::delete_database(db_name).await
    }

    /// Imports from binary format.
    #[wasm_bindgen]
    pub fn import_from_bytes(bytes: &[u8]) -> Result<VectorStore, JsValue> {
        StoreState::import_from_bytes(bytes).map(Self::from_state)
    }

    /// Inserts a sparse vector into the internal sparse index.
    ///
    /// Lazily initializes the sparse index on first call.
    #[wasm_bindgen]
    pub fn sparse_insert(
        &mut self,
        doc_id: u64,
        indices: &[u32],
        values: &[f32],
    ) -> Result<(), JsValue> {
        self.state
            .borrow_mut()
            .sparse_insert(doc_id, indices, values)
    }

    /// Removes a document's sparse postings from the internal sparse index.
    ///
    /// Returns `true` when the document was present; `false` when no sparse
    /// index exists or the document was never inserted.
    #[wasm_bindgen]
    pub fn sparse_remove(&mut self, doc_id: u64) -> bool {
        self.state.borrow_mut().sparse_remove(doc_id)
    }

    /// Searches the internal sparse index.
    ///
    /// Returns a JSON array of `{doc_id, score}` objects sorted by score
    /// descending, `doc_id` a decimal string.
    #[wasm_bindgen]
    pub fn sparse_search(
        &self,
        indices: &[u32],
        values: &[f32],
        k: usize,
    ) -> Result<JsValue, JsValue> {
        self.state.borrow().sparse_search(indices, values, k)
    }
}

#[cfg(test)]
#[path = "vector_store_tests.rs"]
mod tests;
