// WASM bindings have different conventions - relax pedantic/nursery lints for FFI boundary
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::similar_names)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::unused_self)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::manual_let_else)]

//! `VelesDB` WASM - Vector search in the browser
//!
//! This crate provides WebAssembly bindings for `VelesDB`'s core vector operations.
//! It enables browser-based vector search without any server dependency.
//!
//! # Features
//!
//! - **In-memory vector store**: Fast vector storage and retrieval
//! - **SIMD-optimized**: Uses WASM SIMD128 for distance calculations
//! - **Multiple metrics**: Cosine, Euclidean, Dot Product
//! - **Half-precision**: f16/bf16 support for 50% memory reduction
//!
//! # Usage (JavaScript)
//!
//! ```javascript
//! import init, { VectorStore } from 'velesdb-wasm';
//!
//! await init();
//!
//! const store = new VectorStore(768, "cosine");
//! store.insert(1, new Float32Array([0.1, 0.2, ...]));
//! const results = store.search(new Float32Array([0.1, ...]), 10);
//! ```

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

mod agent;
mod distance;
mod filter;
mod fusion;
mod graph;
mod graph_persistence;
mod graph_worker;
mod persistence;
mod serialization;
mod simd;
mod store_get;
mod store_insert;
mod store_new;
mod store_search;
mod text_search;
mod vector_ops;
mod velesql;

pub use agent::SemanticMemory;
pub use distance::DistanceMetric;
pub use graph::{GraphEdge, GraphNode, GraphStore};

/// Query result for multi-model queries (EPIC-031 US-009).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    /// Node/point ID
    pub node_id: u64,
    /// Vector similarity score (if applicable)
    pub vector_score: Option<f32>,
    /// Graph relevance score (if applicable)
    pub graph_score: Option<f32>,
    /// Combined fused score
    pub fused_score: f32,
    /// Variable bindings/payload
    pub bindings: serde_json::Value,
    /// Column data from JOIN (if applicable)
    pub column_data: Option<serde_json::Value>,
}

/// Storage mode for vector quantization.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageMode {
    /// Full f32 precision (4 bytes per dimension)
    #[default]
    Full,
    /// SQ8: 8-bit scalar quantization (1 byte per dimension, 4x compression)
    SQ8,
    /// Binary: 1-bit quantization (1 bit per dimension, 32x compression)
    Binary,
}

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
/// - `Full`: f32 precision, best recall
/// - `SQ8`: 4x memory reduction, ~1% recall loss
/// - `Binary`: 32x memory reduction, ~5-10% recall loss
#[wasm_bindgen]
pub struct VectorStore {
    /// Vector IDs in insertion order
    ids: Vec<u64>,
    /// Contiguous buffer for Full mode (f32)
    data: Vec<f32>,
    /// Contiguous buffer for SQ8 mode (u8)
    data_sq8: Vec<u8>,
    /// Contiguous buffer for Binary mode (packed bits)
    data_binary: Vec<u8>,
    /// Min values for SQ8 dequantization (per vector)
    sq8_mins: Vec<f32>,
    /// Scale values for SQ8 dequantization (per vector)
    sq8_scales: Vec<f32>,
    /// Payloads (JSON metadata) for each vector
    payloads: Vec<Option<serde_json::Value>>,
    dimension: usize,
    metric: DistanceMetric,
    storage_mode: StorageMode,
}

#[wasm_bindgen]
impl VectorStore {
    /// Creates a new vector store. Metrics: cosine, euclidean, dot, hamming, jaccard.
    #[wasm_bindgen(constructor)]
    pub fn new(dimension: usize, metric: &str) -> Result<VectorStore, JsValue> {
        let metric = store_new::parse_metric(metric)?;
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

    /// Creates store with mode: full (4B/dim), sq8 (4x compression), binary (32x).
    #[wasm_bindgen]
    pub fn new_with_mode(
        dimension: usize,
        metric: &str,
        mode: &str,
    ) -> Result<VectorStore, JsValue> {
        let metric = store_new::parse_metric(metric)?;
        let storage_mode = store_new::parse_storage_mode(mode)?;
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
        store_search::search_with_filter_impl(
            query,
            &self.ids,
            &self.payloads,
            &self.data,
            &self.data_sq8,
            &self.data_binary,
            &self.sq8_mins,
            &self.sq8_scales,
            self.dimension,
            self.metric,
            self.storage_mode,
            k,
            |payload: &serde_json::Value| filter::matches_filter(payload, &filter_obj),
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
        store_search::search(
            query,
            &self.ids,
            &self.data,
            &self.data_sq8,
            &self.data_binary,
            &self.sq8_mins,
            &self.sq8_scales,
            self.dimension,
            self.metric,
            self.storage_mode,
            k,
        )
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
            &self.data_sq8,
            &self.data_binary,
            &self.sq8_mins,
            &self.sq8_scales,
            self.dimension,
            self.metric,
            self.storage_mode,
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
    #[wasm_bindgen]
    pub fn hybrid_search(
        &self,
        query_vector: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        store_search::validate_dimension(query_vector.len(), self.dimension)?;
        if self.storage_mode != StorageMode::Full {
            return self.search(query_vector, k);
        }
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

        // Use compute_scores to get (id, score) pairs
        let mut scores = vector_ops::compute_scores(
            query_vector,
            &self.ids,
            &self.data,
            &self.data_sq8,
            &self.data_binary,
            &self.sq8_mins,
            &self.sq8_scales,
            self.dimension,
            self.metric,
            self.storage_mode,
        );

        // Sort by score
        vector_ops::sort_results(&mut scores, self.metric.higher_is_better());
        scores.truncate(k);

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

    /// Multi-query search with fusion. Strategies: average, maximum, rrf.
    #[wasm_bindgen]
    pub fn multi_query_search(
        &mut self,
        vectors: &[f32],
        num_vectors: usize,
        k: usize,
        strategy: &str,
        rrf_k: Option<u32>,
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
            &self.data_sq8,
            &self.data_binary,
            &self.sq8_mins,
            &self.sq8_scales,
            self.dimension,
            self.metric,
            self.storage_mode,
            k,
            strategy,
            rrf_k,
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
        let expected_len = num_vectors * self.dimension;
        if vectors.len() != expected_len {
            return Err(JsValue::from_str(&format!(
                "Expected {expected_len} floats, got {}",
                vectors.len()
            )));
        }
        if self.storage_mode != StorageMode::Full {
            return Err(JsValue::from_str(
                "batch_search only supports Full storage mode",
            ));
        }
        store_search::batch_search_impl(
            vectors,
            num_vectors,
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

    /// Clears all vectors from the store.
    #[wasm_bindgen]
    pub fn clear(&mut self) {
        self.ids.clear();
        self.data.clear();
        self.data_sq8.clear();
        self.data_binary.clear();
        self.sq8_mins.clear();
        self.sq8_scales.clear();
        self.payloads.clear();
    }

    /// Returns memory usage estimate in bytes.
    #[wasm_bindgen]
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let id_bytes = self.ids.len() * std::mem::size_of::<u64>();
        match self.storage_mode {
            StorageMode::Full => id_bytes + self.data.len() * 4,
            StorageMode::SQ8 => {
                id_bytes + self.data_sq8.len() + (self.sq8_mins.len() + self.sq8_scales.len()) * 4
            }
            StorageMode::Binary => id_bytes + self.data_binary.len(),
        }
    }

    /// Creates store with pre-allocated capacity.
    #[wasm_bindgen]
    pub fn with_capacity(
        dimension: usize,
        metric: &str,
        capacity: usize,
    ) -> Result<VectorStore, JsValue> {
        let metric = store_new::parse_metric(metric)?;
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
        self.data.reserve(additional * self.dimension);
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
        self.data.reserve(batch.len() * self.dimension);
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

    /// Exports to binary format for IndexedDB/localStorage.
    #[wasm_bindgen]
    pub fn export_to_bytes(&self) -> Result<Vec<u8>, JsValue> {
        Ok(serialization::export_to_bytes(self))
    }

    /// Saves to `IndexedDB`.
    #[wasm_bindgen]
    pub async fn save(&self, db_name: &str) -> Result<(), JsValue> {
        let bytes = self.export_to_bytes()?;
        persistence::save_to_indexeddb(db_name, &bytes).await
    }

    /// Loads from `IndexedDB`.
    #[wasm_bindgen]
    pub async fn load(db_name: &str) -> Result<VectorStore, JsValue> {
        let bytes = persistence::load_from_indexeddb(db_name).await?;
        Self::import_from_bytes(&bytes)
    }

    /// Deletes `IndexedDB` database.
    #[wasm_bindgen]
    pub async fn delete_database(db_name: &str) -> Result<(), JsValue> {
        persistence::delete_database(db_name).await
    }

    /// Imports from binary format.
    #[wasm_bindgen]
    pub fn import_from_bytes(bytes: &[u8]) -> Result<VectorStore, JsValue> {
        serialization::import_from_bytes(bytes)
    }
}

/// Search result containing ID and score.
#[derive(Serialize, Deserialize)]
pub struct SearchResult {
    pub id: u64,
    pub score: f32,
}

// Console logging for debugging
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[allow(unused_macros)]
macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
