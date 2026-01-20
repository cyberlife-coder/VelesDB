// WASM bindings have different conventions - relax pedantic lints for FFI boundary
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

//! VelesDB WASM - Browser-based vector search with SIMD optimization.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

mod distance;
mod filter;
mod fusion;
mod graph;
mod persistence;
mod quantization;
mod search;
mod serialization;
mod simd;
mod text_search;
mod vector_ops;

pub use distance::DistanceMetric;
pub use graph::{GraphEdge, GraphNode, GraphStore};

/// Parses metric string to DistanceMetric enum.
fn parse_metric(metric: &str) -> Result<DistanceMetric, &'static str> {
    match metric.to_lowercase().as_str() {
        "cosine" => Ok(DistanceMetric::Cosine),
        "euclidean" | "l2" => Ok(DistanceMetric::Euclidean),
        "dot" | "dotproduct" | "inner" => Ok(DistanceMetric::DotProduct),
        "hamming" => Ok(DistanceMetric::Hamming),
        "jaccard" => Ok(DistanceMetric::Jaccard),
        _ => Err("Unknown metric. Use: cosine, euclidean, dot, hamming, jaccard"),
    }
}

/// Parses storage mode string to StorageMode enum.
fn parse_storage_mode(mode: &str) -> Result<StorageMode, &'static str> {
    match mode.to_lowercase().as_str() {
        "full" => Ok(StorageMode::Full),
        "sq8" => Ok(StorageMode::SQ8),
        "binary" => Ok(StorageMode::Binary),
        _ => Err("Unknown storage mode. Use: full, sq8, binary"),
    }
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

/// In-memory vector store with contiguous layout for optimal performance.
#[wasm_bindgen]
pub struct VectorStore {
    ids: Vec<u64>,
    data: Vec<f32>,
    data_sq8: Vec<u8>,
    data_binary: Vec<u8>,
    sq8_mins: Vec<f32>,
    sq8_scales: Vec<f32>,
    payloads: Vec<Option<serde_json::Value>>,
    dimension: usize,
    metric: DistanceMetric,
    storage_mode: StorageMode,
}

#[wasm_bindgen]
impl VectorStore {
    /// Creates a new vector store with specified dimension and metric.
    #[wasm_bindgen(constructor)]
    pub fn new(dimension: usize, metric: &str) -> Result<VectorStore, JsValue> {
        let metric = parse_metric(metric).map_err(JsValue::from_str)?;
        Ok(Self::create_empty(dimension, metric, StorageMode::Full, 0))
    }

    /// Internal helper to create an empty store with specified parameters.
    fn create_empty(
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        capacity: usize,
    ) -> Self {
        Self {
            ids: Vec::with_capacity(capacity),
            data: Vec::with_capacity(capacity * dimension),
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: Vec::with_capacity(capacity),
            dimension,
            metric,
            storage_mode,
        }
    }

    /// Creates a metadata-only store (no vectors, only payloads).
    #[wasm_bindgen]
    pub fn new_metadata_only() -> VectorStore {
        Self::create_empty(0, DistanceMetric::Cosine, StorageMode::Full, 0)
    }

    /// Returns true if this is a metadata-only store.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.dimension == 0
    }

    /// Creates a new vector store with specified storage mode for memory optimization.
    ///
    /// Storage modes: `full` (4B/dim), `sq8` (1B/dim, 4x compression), `binary` (1bit/dim, 32x)
    #[wasm_bindgen]
    pub fn new_with_mode(
        dimension: usize,
        metric: &str,
        mode: &str,
    ) -> Result<VectorStore, JsValue> {
        let metric = parse_metric(metric).map_err(JsValue::from_str)?;
        let storage_mode = parse_storage_mode(mode).map_err(JsValue::from_str)?;
        Ok(Self::create_empty(dimension, metric, storage_mode, 0))
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
        if vector.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            )));
        }

        if let Some(idx) = self.ids.iter().position(|&x| x == id) {
            self.remove_at_index(idx);
        }

        self.ids.push(id);
        self.payloads.push(None);
        self.append_vector_data(vector);
        Ok(())
    }

    /// Inserts a vector with the given ID and optional JSON payload.
    #[wasm_bindgen]
    pub fn insert_with_payload(
        &mut self,
        id: u64,
        vector: &[f32],
        payload: JsValue,
    ) -> Result<(), JsValue> {
        if vector.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            )));
        }

        let parsed_payload: Option<serde_json::Value> =
            if payload.is_null() || payload.is_undefined() {
                None
            } else {
                Some(
                    serde_wasm_bindgen::from_value(payload)
                        .map_err(|e| JsValue::from_str(&format!("Invalid payload: {e}")))?,
                )
            };

        if let Some(idx) = self.ids.iter().position(|&x| x == id) {
            self.remove_at_index(idx);
        }

        self.ids.push(id);
        self.payloads.push(parsed_payload);
        self.append_vector_data(vector);
        Ok(())
    }

    /// Gets a vector by ID. Returns null if not found.
    #[wasm_bindgen]
    pub fn get(&self, id: u64) -> Result<JsValue, JsValue> {
        let idx = match self.ids.iter().position(|&x| x == id) {
            Some(i) => i,
            None => return Ok(JsValue::NULL),
        };

        let vector = self.get_vector_at_index(idx);
        let result = serde_json::json!({"id": id, "vector": vector, "payload": self.payloads[idx]});
        serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Searches with metadata filtering.
    #[wasm_bindgen]
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: JsValue,
    ) -> Result<JsValue, JsValue> {
        if query.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimension,
                query.len()
            )));
        }

        let filter_obj: serde_json::Value = serde_wasm_bindgen::from_value(filter)
            .map_err(|e| JsValue::from_str(&format!("Invalid filter: {e}")))?;

        let store_ref = self.as_store_ref();
        let results = search::search_with_filter_impl(&store_ref, query, k, &filter_obj);

        let output: Vec<serde_json::Value> = results
            .into_iter()
            .map(|(id, score, payload)| serde_json::json!({"id": id, "score": score, "payload": payload}))
            .collect();
        serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Removes vector at the given index (internal helper).
    fn remove_at_index(&mut self, idx: usize) {
        self.ids.remove(idx);
        self.payloads.remove(idx);
        match self.storage_mode {
            StorageMode::Full => {
                let start = idx * self.dimension;
                self.data.drain(start..start + self.dimension);
            }
            StorageMode::SQ8 => {
                let start = idx * self.dimension;
                self.data_sq8.drain(start..start + self.dimension);
                self.sq8_mins.remove(idx);
                self.sq8_scales.remove(idx);
            }
            StorageMode::Binary => {
                let bytes_per_vec = self.dimension.div_ceil(8);
                let start = idx * bytes_per_vec;
                self.data_binary.drain(start..start + bytes_per_vec);
            }
        }
    }

    /// Creates a reference to store data for search operations.
    fn as_store_ref(&self) -> search::StoreRef<'_> {
        search::StoreRef {
            ids: &self.ids,
            data: &self.data,
            data_sq8: &self.data_sq8,
            data_binary: &self.data_binary,
            sq8_mins: &self.sq8_mins,
            sq8_scales: &self.sq8_scales,
            payloads: &self.payloads,
            dimension: self.dimension,
            metric: &self.metric,
            storage_mode: self.storage_mode,
        }
    }

    /// Appends vector data to the appropriate storage buffer based on storage mode.
    fn append_vector_data(&mut self, vector: &[f32]) {
        match self.storage_mode {
            StorageMode::Full => self.data.extend_from_slice(vector),
            StorageMode::SQ8 => {
                let (quantized, min, scale) = quantization::quantize_sq8(vector);
                self.sq8_mins.push(min);
                self.sq8_scales.push(scale);
                self.data_sq8.extend(quantized);
            }
            StorageMode::Binary => {
                let packed = quantization::pack_binary(vector, self.dimension);
                self.data_binary.extend(packed);
            }
        }
    }

    /// Gets vector data at the given index, dequantizing if necessary.
    fn get_vector_at_index(&self, idx: usize) -> Vec<f32> {
        match self.storage_mode {
            StorageMode::Full => {
                let start = idx * self.dimension;
                self.data[start..start + self.dimension].to_vec()
            }
            StorageMode::SQ8 => {
                let start = idx * self.dimension;
                quantization::dequantize_sq8(
                    &self.data_sq8[start..start + self.dimension],
                    self.sq8_mins[idx],
                    self.sq8_scales[idx],
                )
            }
            StorageMode::Binary => {
                let bytes_per_vec = self.dimension.div_ceil(8);
                let start = idx * bytes_per_vec;
                quantization::unpack_binary(
                    &self.data_binary[start..start + bytes_per_vec],
                    self.dimension,
                )
            }
        }
    }

    /// Searches for the k nearest neighbors to the query vector.
    #[wasm_bindgen]
    pub fn search(&self, query: &[f32], k: usize) -> Result<JsValue, JsValue> {
        if query.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimension,
                query.len()
            )));
        }

        let store_ref = self.as_store_ref();
        let results = search::search_knn(&store_ref, query, k);
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Similarity search with threshold filtering (VelesQL equivalent).
    #[wasm_bindgen]
    pub fn similarity_search(
        &self,
        query: &[f32],
        threshold: f32,
        operator: &str,
        k: usize,
    ) -> Result<JsValue, JsValue> {
        if query.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimension,
                query.len()
            )));
        }

        let op_fn = search::parse_similarity_operator(operator).ok_or_else(|| {
            JsValue::from_str(
                "Invalid operator. Use: >, >=, <, <=, =, != (or gt, gte, lt, lte, eq, neq)",
            )
        })?;

        let store_ref = self.as_store_ref();
        let results = search::similarity_search_impl(&store_ref, query, threshold, &*op_fn, k);
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Performs text search on payload fields.
    #[wasm_bindgen]
    pub fn text_search(
        &self,
        query: &str,
        k: usize,
        field: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let results =
            search::text_search_impl(&self.ids, &self.payloads, query, k, field.as_deref());
        let output: Vec<serde_json::Value> = results
            .into_iter()
            .map(|(id, score, payload)| serde_json::json!({"id": id, "score": score, "payload": payload}))
            .collect();
        serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Hybrid search combining vector similarity and text search.
    #[wasm_bindgen]
    pub fn hybrid_search(
        &self,
        query_vector: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        if query_vector.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                query_vector.len()
            )));
        }

        // SQ8/Binary fallback to simple vector search
        if self.storage_mode != StorageMode::Full {
            return self.search(query_vector, k);
        }

        let v_weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let store_ref = self.as_store_ref();
        let results = search::hybrid_search_impl(&store_ref, query_vector, text_query, k, v_weight);

        let output: Vec<serde_json::Value> = results
            .into_iter()
            .map(|(id, score, payload)| serde_json::json!({"id": id, "score": score, "payload": payload}))
            .collect();
        serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Multi-query search with result fusion (average/maximum/rrf).
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
        let expected_len = num_vectors * self.dimension;
        if vectors.len() != expected_len {
            return Err(JsValue::from_str(&format!(
                "Expected {} floats ({} vectors × {} dims), got {}",
                expected_len,
                num_vectors,
                self.dimension,
                vectors.len()
            )));
        }

        let store_ref = self.as_store_ref();
        let results = search::multi_query_search_impl(
            &store_ref,
            vectors,
            num_vectors,
            k,
            strategy,
            rrf_k.unwrap_or(60),
        );
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Batch search for multiple vectors.
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
                "Expected {} floats, got {}",
                expected_len,
                vectors.len()
            )));
        }

        let store_ref = self.as_store_ref();
        let results = search::batch_search_impl(&store_ref, vectors, num_vectors, k);
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
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

    /// Creates a new vector store with pre-allocated capacity.
    #[wasm_bindgen]
    pub fn with_capacity(
        dimension: usize,
        metric: &str,
        capacity: usize,
    ) -> Result<VectorStore, JsValue> {
        let metric = parse_metric(metric).map_err(JsValue::from_str)?;
        Ok(Self::create_empty(
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

    /// Inserts multiple vectors in a single batch operation.
    #[wasm_bindgen]
    pub fn insert_batch(&mut self, batch: JsValue) -> Result<(), JsValue> {
        let batch: Vec<(u64, Vec<f32>)> = serde_wasm_bindgen::from_value(batch)
            .map_err(|e| JsValue::from_str(&format!("Invalid batch format: {e}")))?;

        // Validate all dimensions first
        for (id, vector) in &batch {
            if vector.len() != self.dimension {
                return Err(JsValue::from_str(&format!(
                    "Vector {} dimension mismatch: expected {}, got {}",
                    id,
                    self.dimension,
                    vector.len()
                )));
            }
        }

        // Pre-allocate space for contiguous buffer
        self.ids.reserve(batch.len());
        self.data.reserve(batch.len() * self.dimension);

        // Remove existing IDs first
        let ids_to_remove: Vec<u64> = batch.iter().map(|(id, _)| *id).collect();
        for id in &ids_to_remove {
            if let Some(idx) = self.ids.iter().position(|&x| x == *id) {
                self.ids.remove(idx);
                let start = idx * self.dimension;
                self.data.drain(start..start + self.dimension);
            }
        }

        // Insert all vectors into contiguous buffer
        for (id, vector) in batch {
            self.ids.push(id);
            self.data.extend_from_slice(&vector);
        }

        Ok(())
    }

    /// Exports the vector store to a binary format for persistence.
    #[wasm_bindgen]
    pub fn export_to_bytes(&self) -> Result<Vec<u8>, JsValue> {
        Ok(serialization::export_to_bytes(
            &self.ids,
            &self.data,
            self.dimension,
            &self.metric,
        ))
    }

    /// Saves the vector store to `IndexedDB`.
    #[wasm_bindgen]
    pub async fn save(&self, db_name: &str) -> Result<(), JsValue> {
        let bytes = self.export_to_bytes()?;
        persistence::save_to_indexeddb(db_name, &bytes).await
    }

    /// Loads a vector store from `IndexedDB`.
    #[wasm_bindgen]
    pub async fn load(db_name: &str) -> Result<VectorStore, JsValue> {
        let bytes = persistence::load_from_indexeddb(db_name).await?;
        Self::import_from_bytes(&bytes)
    }

    /// Deletes the `IndexedDB` database.
    #[wasm_bindgen]
    pub async fn delete_database(db_name: &str) -> Result<(), JsValue> {
        persistence::delete_database(db_name).await
    }

    /// Imports a vector store from binary format.
    #[wasm_bindgen]
    pub fn import_from_bytes(bytes: &[u8]) -> Result<VectorStore, JsValue> {
        let header = serialization::parse_header(bytes).map_err(JsValue::from_str)?;
        let (ids, data) = serialization::import_data(bytes, &header);
        let count = header.count;

        Ok(Self {
            ids,
            data,
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: vec![None; count],
            dimension: header.dimension,
            metric: header.metric,
            storage_mode: StorageMode::Full,
        })
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
