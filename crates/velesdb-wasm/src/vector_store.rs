//! `VectorStore` — WASM-exported vector store for in-browser vector search.

use wasm_bindgen::prelude::*;

use crate::filter;
use crate::parsing;
use crate::sparse;
use crate::store_get;
use crate::store_insert;
use crate::store_new;
use crate::store_search;
use crate::{DistanceMetric, QueryResult, StorageMode};

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
#[wasm_bindgen]
pub struct VectorStore {
    /// Vector IDs in insertion order
    pub(crate) ids: Vec<u64>,
    /// Contiguous f32 vectors, `dimension` floats per id, in every storage mode
    pub(crate) data: Vec<f32>,
    /// Payloads (JSON metadata) for each vector
    pub(crate) payloads: Vec<Option<serde_json::Value>>,
    /// Number of dimensions per vector
    pub(crate) dimension: usize,
    /// Distance metric used for similarity computation
    pub(crate) metric: DistanceMetric,
    /// Declared storage mode (accepted, persisted and reported; see [`StorageMode`])
    pub(crate) storage_mode: StorageMode,
    /// Optional sparse index for sparse/hybrid search
    pub(crate) sparse_index: Option<sparse::SparseIndex>,
}

impl VectorStore {
    /// Returns the JSON payload stored for `id`, if the id exists and carries one.
    ///
    /// Used by cross-collection MATCH enrichment to merge a referenced
    /// collection's metadata into graph results by node id.
    pub(crate) fn payload_for_id(&self, id: u64) -> Option<&serde_json::Value> {
        let idx = self.ids.iter().position(|&x| x == id)?;
        self.payloads.get(idx).and_then(Option::as_ref)
    }
}

#[wasm_bindgen]
impl VectorStore {
    /// Creates a new vector store. Metrics: cosine, euclidean, dot, hamming, jaccard.
    #[wasm_bindgen(constructor)]
    pub fn new(dimension: usize, metric: &str) -> Result<VectorStore, JsValue> {
        let metric = parsing::parse_metric(metric)?;
        Ok(store_new::create_store(
            dimension,
            metric,
            StorageMode::Full,
        ))
    }

    /// Creates a metadata-only store (no vectors, only payloads).
    #[wasm_bindgen]
    pub fn new_metadata_only() -> VectorStore {
        store_new::create_metadata_only()
    }

    /// Returns true if this is a metadata-only store.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.dimension == 0
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
        let metric = parsing::parse_metric(metric)?;
        let storage_mode = parsing::parse_storage_mode(mode)?;
        Ok(store_new::create_store(dimension, metric, storage_mode))
    }

    /// Returns the storage mode.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn storage_mode(&self) -> String {
        match self.storage_mode {
            StorageMode::Full => "full".to_string(),
            StorageMode::SQ8 => "sq8".to_string(),
            StorageMode::Binary => "binary".to_string(),
            StorageMode::ProductQuantization => "pq".to_string(),
            StorageMode::RaBitQ => "rabitq".to_string(),
        }
    }

    /// Returns the number of vectors in the store.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Returns true if the store is empty.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Returns the vector dimension.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Inserts a vector with the given ID.
    #[wasm_bindgen]
    pub fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), JsValue> {
        store_search::validate_dimension(vector.len(), self.dimension)?;
        store_insert::insert_vector(self, id, vector);
        Ok(())
    }

    /// Inserts a vector with ID and optional JSON payload.
    #[wasm_bindgen]
    pub fn insert_with_payload(
        &mut self,
        id: u64,
        vector: &[f32],
        payload: JsValue,
    ) -> Result<(), JsValue> {
        store_search::validate_dimension(vector.len(), self.dimension)?;
        let parsed_payload: Option<serde_json::Value> =
            if payload.is_null() || payload.is_undefined() {
                None
            } else {
                Some(
                    serde_wasm_bindgen::from_value(payload)
                        .map_err(|e| JsValue::from_str(&format!("Invalid payload: {e}")))?,
                )
            };
        store_insert::insert_with_payload(self, id, vector, parsed_payload);
        Ok(())
    }

    /// Gets a vector by ID. Returns {id, vector, payload} or null.
    #[wasm_bindgen]
    pub fn get(&self, id: u64) -> Result<JsValue, JsValue> {
        store_get::get_by_id(self, id)
    }

    /// Searches with metadata filtering. Returns [{id, score, payload}].
    #[wasm_bindgen]
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: JsValue,
    ) -> Result<JsValue, JsValue> {
        store_search::validate_dimension(query.len(), self.dimension)?;
        let filter_obj: serde_json::Value = serde_wasm_bindgen::from_value(filter)
            .map_err(|e| JsValue::from_str(&format!("Invalid filter: {e}")))?;
        let filter = filter::parse_filter(filter_obj).map_err(|e| JsValue::from_str(&e))?;
        store_search::search_with_filter_impl(
            query,
            &self.ids,
            &self.payloads,
            &self.data,
            self.dimension,
            self.metric,
            k,
            |payload: &serde_json::Value| filter.matches(payload),
        )
    }

    /// Removes vector at the given index (internal helper).
    fn remove_at_index(&mut self, idx: usize) {
        store_insert::remove_at_index(self, idx);
    }

    /// k-NN search. Returns [[id, score], ...].
    #[wasm_bindgen]
    pub fn search(&self, query: &[f32], k: usize) -> Result<JsValue, JsValue> {
        store_search::validate_dimension(query.len(), self.dimension)?;
        store_search::search(query, &self.ids, &self.data, self.dimension, self.metric, k)
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
    /// Returns `[[id, score], ...]` sorted by relevance.
    #[wasm_bindgen]
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: &str,
    ) -> Result<JsValue, JsValue> {
        store_search::validate_dimension(query.len(), self.dimension)?;
        // Validate the quality string for API parity. The value is unused
        // because WASM search is brute-force (no HNSW graph).
        parsing::parse_search_quality(quality)?;
        store_search::search(query, &self.ids, &self.data, self.dimension, self.metric, k)
    }

    /// Similarity search with threshold. Operators: >, >=, <, <=, =, !=.
    #[wasm_bindgen]
    pub fn similarity_search(
        &self,
        query: &[f32],
        threshold: f32,
        operator: &str,
        k: usize,
    ) -> Result<JsValue, JsValue> {
        store_search::validate_dimension(query.len(), self.dimension)?;
        store_search::similarity_search_impl(
            query,
            &self.ids,
            &self.data,
            self.dimension,
            self.metric,
            threshold,
            operator,
            k,
        )
    }

    /// Text search on payload fields (substring matching).
    #[wasm_bindgen]
    pub fn text_search(
        &self,
        query: &str,
        k: usize,
        field: Option<String>,
    ) -> Result<JsValue, JsValue> {
        store_search::text_search_impl(query, &self.ids, &self.payloads, field.as_deref(), k)
    }

    /// Hybrid search (vector + text). `vector_weight` 0-1 (default 0.5).
    ///
    /// Computes the exact per-vector score and the text match in a single
    /// pass, in every storage mode.
    ///
    /// Returns `[{id, score, payload}, ...]` sorted by combined score
    /// descending, truncated to `k` results.
    #[wasm_bindgen]
    pub fn hybrid_search(
        &self,
        query_vector: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        store_search::validate_dimension(query_vector.len(), self.dimension)?;
        store_search::hybrid_search_impl(
            query_vector,
            text_query,
            &self.ids,
            &self.data,
            &self.payloads,
            self.dimension,
            self.metric,
            k,
            vector_weight,
        )
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
    /// descending, truncated to `k`.
    #[wasm_bindgen]
    pub fn search_sparse(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<JsValue, JsValue> {
        let results = self
            .search_sparse_scored(query_indices, query_values, k)
            .map_err(|e| JsValue::from_str(&e))?;
        serde_wasm_bindgen::to_value(&results)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
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
    /// Array of `{nodeId, vectorScore, graphScore, fusedScore, bindings, columnData}`
    #[wasm_bindgen]
    pub fn query(&self, query_vector: &[f32], k: usize) -> Result<JsValue, JsValue> {
        store_search::validate_dimension(query_vector.len(), self.dimension)?;

        let scores = self.search_scored(query_vector, k);

        // Build id->index map for payload lookup
        let id_to_idx: std::collections::HashMap<u64, usize> = self
            .ids
            .iter()
            .enumerate()
            .map(|(idx, &id)| (id, idx))
            .collect();

        // Convert to HybridResult format
        let hybrid_results: Vec<QueryResult> = scores
            .into_iter()
            .map(|(id, score)| {
                let payload = id_to_idx
                    .get(&id)
                    .and_then(|&idx| self.payloads.get(idx).cloned().flatten());
                QueryResult {
                    node_id: id,
                    vector_score: Some(score),
                    graph_score: None,
                    fused_score: score,
                    bindings: payload.unwrap_or(serde_json::Value::Null),
                    column_data: None,
                }
            })
            .collect();

        serde_wasm_bindgen::to_value(&hybrid_results)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {e}")))
    }

    /// Multi-query search with fusion. Strategies: average, maximum, weighted, rrf, relative_score/rsf.
    ///
    /// `weights` is an optional `[avg_weight, max_weight, hit_weight]` triple
    /// used only by the `"weighted"` strategy. When omitted, core's canonical
    /// default weights are used (`avg=0.6, max=0.3, hit=0.1`) — see issue
    /// #1545. Passing a slice of any length other than 3 is an error.
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
        if num_vectors == 0 {
            return Err(JsValue::from_str(
                "multi_query_search requires at least one vector",
            ));
        }
        store_search::multi_query_search_impl(
            vectors,
            num_vectors,
            &self.ids,
            &self.data,
            self.dimension,
            self.metric,
            k,
            strategy,
            rrf_k,
            weights.as_deref(),
        )
    }

    /// Batch search for multiple vectors. Returns [[[id, score], ...], ...].
    #[wasm_bindgen]
    pub fn batch_search(
        &self,
        vectors: &[f32],
        num_vectors: usize,
        k: usize,
    ) -> Result<JsValue, JsValue> {
        if num_vectors == 0 {
            return serde_wasm_bindgen::to_value::<Vec<Vec<(u64, f32)>>>(&vec![])
                .map_err(|e| JsValue::from_str(&e.to_string()));
        }
        store_search::validate_multi_vector_len(vectors.len(), num_vectors, self.dimension)
            .map_err(|e| JsValue::from_str(&e))?;
        store_search::batch_search_impl(
            vectors,
            &self.ids,
            &self.data,
            self.dimension,
            self.metric,
            k,
        )
    }

    /// Removes a vector by ID.
    #[wasm_bindgen]
    pub fn remove(&mut self, id: u64) -> bool {
        if let Some(idx) = self.ids.iter().position(|&x| x == id) {
            self.remove_at_index(idx);
            true
        } else {
            false
        }
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
        self.ids.clear();
        self.data.clear();
        self.payloads.clear();
        self.sparse_index = None;
    }

    /// Returns memory usage estimate in bytes: ids plus the f32 vectors,
    /// which every storage mode keeps (see [`StorageMode`]).
    #[wasm_bindgen]
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.ids.len() * std::mem::size_of::<u64>() + std::mem::size_of_val(self.data.as_slice())
    }

    /// Creates store with pre-allocated capacity.
    #[wasm_bindgen]
    pub fn with_capacity(
        dimension: usize,
        metric: &str,
        capacity: usize,
    ) -> Result<VectorStore, JsValue> {
        let metric = parsing::parse_metric(metric)?;
        Ok(store_new::create_with_capacity(
            dimension,
            metric,
            StorageMode::Full,
            capacity,
        ))
    }

    /// Pre-allocates memory for additional vectors.
    #[wasm_bindgen]
    pub fn reserve(&mut self, additional: usize) {
        self.ids.reserve(additional);
        // `reserve` is a capacity hint, so a `saturating_mul` (rather than a
        // wrapping `*`) keeps a 32-bit-usize wasm32 build from silently
        // under-reserving on a huge `additional`; a genuinely oversized request
        // still aborts on allocation, which is the caller's explicit ask.
        self.data.reserve(additional.saturating_mul(self.dimension));
    }

    /// Batch insert. Input: `[[id, Float32Array], ...]`.
    #[wasm_bindgen]
    pub fn insert_batch(&mut self, batch: JsValue) -> Result<(), JsValue> {
        let batch: Vec<(u64, Vec<f32>)> = serde_wasm_bindgen::from_value(batch)
            .map_err(|e| JsValue::from_str(&format!("Invalid batch format: {e}")))?;
        for (id, vector) in &batch {
            if vector.len() != self.dimension {
                return Err(JsValue::from_str(&format!(
                    "Vector {id} dimension mismatch: expected {}, got {}",
                    self.dimension,
                    vector.len()
                )));
            }
        }
        self.ids.reserve(batch.len());
        self.data
            .reserve(batch.len().saturating_mul(self.dimension));
        for (id, vector) in batch {
            if let Some(idx) = self.ids.iter().position(|&x| x == id) {
                self.remove_at_index(idx);
            }
            self.ids.push(id);
            self.data.extend_from_slice(&vector);
            self.payloads.push(None);
        }
        Ok(())
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
        store_insert::insert_batch_raw(self, ids, vectors, dimension)
            .map_err(|e| JsValue::from_str(&e))
    }
}

// Native-testable internals (not part of the wasm-bindgen surface).
impl VectorStore {
    /// Top-`k` `(id, score)` pairs for `query`, best first — the exact f32
    /// scores behind [`search`](Self::search) in every storage mode.
    pub(crate) fn search_scored(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        store_search::top_k(query, &self.ids, &self.data, self.dimension, self.metric, k)
    }

    /// Scoring entry point behind [`search_sparse`](Self::search_sparse).
    ///
    /// Returns an error when no sparse index has been built (parity with the
    /// core `sparse_search` contract); otherwise delegates scoring to the
    /// [`sparse::SparseIndex`] kernel. Returns `String` errors and plain
    /// result structs so it can be exercised off-`wasm32`.
    pub(crate) fn search_sparse_scored(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<Vec<sparse::SparseSearchResult>, String> {
        let idx = self
            .sparse_index
            .as_ref()
            .ok_or_else(|| "sparse index not found: insert sparse vectors first".to_string())?;
        idx.search_scored(query_indices, query_values, k)
    }
}
