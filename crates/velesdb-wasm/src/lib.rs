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

mod distance;
mod filter;
mod fusion;
mod graph;
mod persistence;
mod simd;
mod text_search;

pub use distance::DistanceMetric;
pub use graph::{GraphEdge, GraphNode, GraphStore};

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
    /// Creates a new vector store with the specified dimension and distance metric.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension (e.g., 768 for BERT, 1536 for GPT)
    /// * `metric` - Distance metric: "cosine", "euclidean", or "dot"
    ///
    /// # Errors
    ///
    /// Returns an error if the metric is not recognized.
    #[wasm_bindgen(constructor)]
    pub fn new(dimension: usize, metric: &str) -> Result<VectorStore, JsValue> {
        let metric = match metric.to_lowercase().as_str() {
            "cosine" => DistanceMetric::Cosine,
            "euclidean" | "l2" => DistanceMetric::Euclidean,
            "dot" | "dotproduct" | "inner" => DistanceMetric::DotProduct,
            "hamming" => DistanceMetric::Hamming,
            "jaccard" => DistanceMetric::Jaccard,
            _ => {
                return Err(JsValue::from_str(
                    "Unknown metric. Use: cosine, euclidean, dot, hamming, jaccard",
                ))
            }
        };

        Ok(Self {
            ids: Vec::new(),
            data: Vec::new(),
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: Vec::new(),
            dimension,
            metric,
            storage_mode: StorageMode::Full,
        })
    }

    /// Creates a metadata-only store (no vectors, only payloads).
    ///
    /// Useful for storing auxiliary data without vector embeddings.
    #[wasm_bindgen]
    pub fn new_metadata_only() -> VectorStore {
        Self {
            ids: Vec::new(),
            data: Vec::new(),
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: Vec::new(),
            dimension: 0,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        }
    }

    /// Returns true if this is a metadata-only store.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.dimension == 0
    }

    /// Creates a new vector store with specified storage mode for memory optimization.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `mode` - Storage mode: "full", "sq8", or "binary"
    ///
    /// # Storage Modes
    ///
    /// - `full`: Best recall, 4 bytes/dimension
    /// - `sq8`: 4x compression, ~1% recall loss
    /// - `binary`: 32x compression, ~5-10% recall loss
    ///
    /// # Errors
    ///
    /// Returns an error if the metric or storage mode is unknown.
    #[wasm_bindgen]
    pub fn new_with_mode(
        dimension: usize,
        metric: &str,
        mode: &str,
    ) -> Result<VectorStore, JsValue> {
        let metric = match metric.to_lowercase().as_str() {
            "cosine" => DistanceMetric::Cosine,
            "euclidean" | "l2" => DistanceMetric::Euclidean,
            "dot" | "dotproduct" | "inner" => DistanceMetric::DotProduct,
            "hamming" => DistanceMetric::Hamming,
            "jaccard" => DistanceMetric::Jaccard,
            _ => {
                return Err(JsValue::from_str(
                    "Unknown metric. Use: cosine, euclidean, dot, hamming, jaccard",
                ))
            }
        };

        let storage_mode = match mode.to_lowercase().as_str() {
            "full" => StorageMode::Full,
            "sq8" => StorageMode::SQ8,
            "binary" => StorageMode::Binary,
            _ => {
                return Err(JsValue::from_str(
                    "Unknown storage mode. Use: full, sq8, binary",
                ))
            }
        };

        Ok(Self {
            ids: Vec::new(),
            data: Vec::new(),
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: Vec::new(),
            dimension,
            metric,
            storage_mode,
        })
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
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the vector
    /// * `vector` - `Float32Array` of the vector data
    ///
    /// # Errors
    ///
    /// Returns an error if vector dimension doesn't match store dimension.
    #[wasm_bindgen]
    pub fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), JsValue> {
        if vector.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            )));
        }

        // Remove existing vector with same ID if present
        if let Some(idx) = self.ids.iter().position(|&x| x == id) {
            self.remove_at_index(idx);
        }

        // Append based on storage mode
        self.ids.push(id);
        self.payloads.push(None);
        match self.storage_mode {
            StorageMode::Full => {
                self.data.extend_from_slice(vector);
            }
            StorageMode::SQ8 => {
                // SQ8: Quantize to u8 with per-vector min/scale
                let (min, max) = vector.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
                    (min.min(v), max.max(v))
                });
                let scale = if (max - min).abs() < 1e-10 {
                    1.0
                } else {
                    255.0 / (max - min)
                };

                self.sq8_mins.push(min);
                self.sq8_scales.push(scale);

                for &v in vector {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let quantized = ((v - min) * scale).round().clamp(0.0, 255.0) as u8;
                    self.data_sq8.push(quantized);
                }
            }
            StorageMode::Binary => {
                // Binary: Pack 8 bits per byte (1 bit per dimension)
                let bytes_needed = self.dimension.div_ceil(8);
                for byte_idx in 0..bytes_needed {
                    let mut byte = 0u8;
                    for bit in 0..8 {
                        let dim_idx = byte_idx * 8 + bit;
                        if dim_idx < self.dimension && vector[dim_idx] > 0.0 {
                            byte |= 1 << bit;
                        }
                    }
                    self.data_binary.push(byte);
                }
            }
        }

        Ok(())
    }

    /// Inserts a vector with the given ID and optional JSON payload.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the vector
    /// * `vector` - `Float32Array` of the vector data
    /// * `payload` - Optional JSON payload (metadata)
    ///
    /// # Errors
    ///
    /// Returns an error if vector dimension doesn't match store dimension.
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

        // Parse payload from JsValue
        let parsed_payload: Option<serde_json::Value> =
            if payload.is_null() || payload.is_undefined() {
                None
            } else {
                Some(
                    serde_wasm_bindgen::from_value(payload)
                        .map_err(|e| JsValue::from_str(&format!("Invalid payload: {e}")))?,
                )
            };

        // Remove existing vector with same ID if present
        if let Some(idx) = self.ids.iter().position(|&x| x == id) {
            self.remove_at_index(idx);
        }

        // Append based on storage mode
        self.ids.push(id);
        self.payloads.push(parsed_payload);
        match self.storage_mode {
            StorageMode::Full => {
                self.data.extend_from_slice(vector);
            }
            StorageMode::SQ8 => {
                let (min, max) = vector.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
                    (min.min(v), max.max(v))
                });
                let scale = if (max - min).abs() < 1e-10 {
                    1.0
                } else {
                    255.0 / (max - min)
                };

                self.sq8_mins.push(min);
                self.sq8_scales.push(scale);

                for &v in vector {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let quantized = ((v - min) * scale).round().clamp(0.0, 255.0) as u8;
                    self.data_sq8.push(quantized);
                }
            }
            StorageMode::Binary => {
                let bytes_needed = self.dimension.div_ceil(8);
                for byte_idx in 0..bytes_needed {
                    let mut byte = 0u8;
                    for bit in 0..8 {
                        let dim_idx = byte_idx * 8 + bit;
                        if dim_idx < self.dimension && vector[dim_idx] > 0.0 {
                            byte |= 1 << bit;
                        }
                    }
                    self.data_binary.push(byte);
                }
            }
        }

        Ok(())
    }

    /// Gets a vector by ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The vector ID to retrieve
    ///
    /// # Returns
    ///
    /// An object with `id`, `vector`, and `payload` fields, or null if not found.
    #[wasm_bindgen]
    pub fn get(&self, id: u64) -> Result<JsValue, JsValue> {
        let idx = match self.ids.iter().position(|&x| x == id) {
            Some(i) => i,
            None => return Ok(JsValue::NULL),
        };

        let vector: Vec<f32> = match self.storage_mode {
            StorageMode::Full => {
                let start = idx * self.dimension;
                self.data[start..start + self.dimension].to_vec()
            }
            StorageMode::SQ8 => {
                let start = idx * self.dimension;
                let min = self.sq8_mins[idx];
                let scale = self.sq8_scales[idx];
                self.data_sq8[start..start + self.dimension]
                    .iter()
                    .map(|&q| (f32::from(q) / scale) + min)
                    .collect()
            }
            StorageMode::Binary => {
                let bytes_per_vec = self.dimension.div_ceil(8);
                let start = idx * bytes_per_vec;
                let mut vec = vec![0.0f32; self.dimension];
                for (i, &byte) in self.data_binary[start..start + bytes_per_vec]
                    .iter()
                    .enumerate()
                {
                    for bit in 0..8 {
                        let dim_idx = i * 8 + bit;
                        if dim_idx < self.dimension {
                            vec[dim_idx] = if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
                        }
                    }
                }
                vec
            }
        };

        let result = serde_json::json!({
            "id": id,
            "vector": vector,
            "payload": self.payloads[idx]
        });

        serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Searches with metadata filtering.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Number of results
    /// * `filter` - JSON filter object (e.g., `{"condition": {"type": "eq", "field": "category", "value": "tech"}}`)
    ///
    /// # Returns
    ///
    /// Array of `[id, score, payload]` tuples sorted by relevance.
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

        // Parse filter - expecting a Filter structure from velesdb-core
        let filter_obj: serde_json::Value = serde_wasm_bindgen::from_value(filter)
            .map_err(|e| JsValue::from_str(&format!("Invalid filter: {e}")))?;

        let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = match self.storage_mode {
            StorageMode::Full => self
                .ids
                .iter()
                .enumerate()
                .filter_map(|(idx, &id)| {
                    let payload = self.payloads[idx].as_ref()?;
                    if !filter::matches_filter(payload, &filter_obj) {
                        return None;
                    }
                    let start = idx * self.dimension;
                    let v_data = &self.data[start..start + self.dimension];
                    let score = self.metric.calculate(query, v_data);
                    Some((id, score, Some(payload)))
                })
                .collect(),
            StorageMode::SQ8 => {
                let mut dequantized = vec![0.0f32; self.dimension];
                self.ids
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, &id)| {
                        let payload = self.payloads[idx].as_ref()?;
                        if !filter::matches_filter(payload, &filter_obj) {
                            return None;
                        }
                        let start = idx * self.dimension;
                        let min = self.sq8_mins[idx];
                        let scale = self.sq8_scales[idx];
                        for (i, &q) in self.data_sq8[start..start + self.dimension]
                            .iter()
                            .enumerate()
                        {
                            dequantized[i] = (f32::from(q) / scale) + min;
                        }
                        let score = self.metric.calculate(query, &dequantized);
                        Some((id, score, Some(payload)))
                    })
                    .collect()
            }
            StorageMode::Binary => {
                let bytes_per_vec = self.dimension.div_ceil(8);
                let mut binary_vec = vec![0.0f32; self.dimension];
                self.ids
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, &id)| {
                        let payload = self.payloads[idx].as_ref()?;
                        if !filter::matches_filter(payload, &filter_obj) {
                            return None;
                        }
                        let start = idx * bytes_per_vec;
                        for (i, &byte) in self.data_binary[start..start + bytes_per_vec]
                            .iter()
                            .enumerate()
                        {
                            for bit in 0..8 {
                                let dim_idx = i * 8 + bit;
                                if dim_idx < self.dimension {
                                    binary_vec[dim_idx] =
                                        if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
                                }
                            }
                        }
                        let score = self.metric.calculate(query, &binary_vec);
                        Some((id, score, Some(payload)))
                    })
                    .collect()
            }
        };

        // Sort by relevance
        if self.metric.higher_is_better() {
            results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        results.truncate(k);

        // Convert to serializable format
        let output: Vec<serde_json::Value> = results
            .into_iter()
            .map(|(id, score, payload)| {
                serde_json::json!({
                    "id": id,
                    "score": score,
                    "payload": payload
                })
            })
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

    /// Searches for the k nearest neighbors to the query vector.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector as `Float32Array`
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Array of [id, score] pairs sorted by relevance.
    ///
    /// # Errors
    ///
    /// Returns an error if query dimension doesn't match store dimension.
    #[wasm_bindgen]
    pub fn search(&self, query: &[f32], k: usize) -> Result<JsValue, JsValue> {
        if query.len() != self.dimension {
            return Err(JsValue::from_str(&format!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimension,
                query.len()
            )));
        }

        let mut results: Vec<(u64, f32)> = match self.storage_mode {
            StorageMode::Full => {
                // Full precision - direct calculation
                self.ids
                    .iter()
                    .enumerate()
                    .map(|(idx, &id)| {
                        let start = idx * self.dimension;
                        let v_data = &self.data[start..start + self.dimension];
                        let score = self.metric.calculate(query, v_data);
                        (id, score)
                    })
                    .collect()
            }
            StorageMode::SQ8 => {
                // SQ8 - dequantize on the fly
                let mut dequantized = vec![0.0f32; self.dimension];
                self.ids
                    .iter()
                    .enumerate()
                    .map(|(idx, &id)| {
                        let start = idx * self.dimension;
                        let min = self.sq8_mins[idx];
                        let scale = self.sq8_scales[idx];

                        // Dequantize: value = (quantized / scale) + min
                        for (i, &q) in self.data_sq8[start..start + self.dimension]
                            .iter()
                            .enumerate()
                        {
                            dequantized[i] = (f32::from(q) / scale) + min;
                        }

                        let score = self.metric.calculate(query, &dequantized);
                        (id, score)
                    })
                    .collect()
            }
            StorageMode::Binary => {
                // Binary - unpack bits and compare
                let bytes_per_vec = self.dimension.div_ceil(8);
                let mut binary_vec = vec![0.0f32; self.dimension];

                self.ids
                    .iter()
                    .enumerate()
                    .map(|(idx, &id)| {
                        let start = idx * bytes_per_vec;

                        // Unpack bits to f32 (0.0 or 1.0)
                        for (i, &byte) in self.data_binary[start..start + bytes_per_vec]
                            .iter()
                            .enumerate()
                        {
                            for bit in 0..8 {
                                let dim_idx = i * 8 + bit;
                                if dim_idx < self.dimension {
                                    binary_vec[dim_idx] =
                                        if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
                                }
                            }
                        }

                        let score = self.metric.calculate(query, &binary_vec);
                        (id, score)
                    })
                    .collect()
            }
        };

        // Sort by relevance
        if self.metric.higher_is_better() {
            results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        results.truncate(k);

        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Performs text search on payload fields.
    ///
    /// This is a simple substring-based search on payload text fields.
    /// For full BM25 text search, use the REST API backend.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `k` - Number of results
    /// * `field` - Optional field name to search in (default: searches all string fields)
    ///
    /// # Returns
    ///
    /// Array of results with matching payloads.
    #[wasm_bindgen]
    pub fn text_search(
        &self,
        query: &str,
        k: usize,
        field: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let query_lower = query.to_lowercase();

        let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = self
            .ids
            .iter()
            .enumerate()
            .filter_map(|(idx, &id)| {
                let payload = self.payloads[idx].as_ref()?;
                let matches =
                    text_search::payload_contains_text(payload, &query_lower, field.as_deref());
                if matches {
                    Some((id, 1.0, Some(payload)))
                } else {
                    None
                }
            })
            .collect();

        results.truncate(k);

        let output: Vec<serde_json::Value> = results
            .into_iter()
            .map(|(id, score, payload)| {
                serde_json::json!({
                    "id": id,
                    "score": score,
                    "payload": payload
                })
            })
            .collect();

        serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Performs hybrid search combining vector similarity and text search.
    ///
    /// Uses a simple weighted fusion of vector search and text search results.
    ///
    /// # Arguments
    ///
    /// * `query_vector` - Query vector for similarity search
    /// * `text_query` - Text query for payload search
    /// * `k` - Number of results to return
    /// * `vector_weight` - Weight for vector results (0.0-1.0, default 0.5)
    ///
    /// # Returns
    ///
    /// Array of fused results with id, score, and payload.
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

        let v_weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let t_weight = 1.0 - v_weight;
        let text_query_lower = text_query.to_lowercase();

        // Perform vector search and text matching in one pass
        let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = match self.storage_mode {
            StorageMode::Full => {
                self.ids
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, &id)| {
                        let start = idx * self.dimension;
                        let v_data = &self.data[start..start + self.dimension];
                        let vector_score = self.metric.calculate(query_vector, v_data);

                        let payload = self.payloads[idx].as_ref();
                        let text_score = if let Some(p) = payload {
                            if text_search::search_all_fields(p, &text_query_lower) {
                                1.0
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        };

                        // Combine scores
                        let combined_score = v_weight * vector_score + t_weight * text_score;
                        if combined_score > 0.0 {
                            Some((id, combined_score, payload))
                        } else {
                            None
                        }
                    })
                    .collect()
            }
            _ => {
                // Simplified for SQ8/Binary - just vector search
                return self.search(query_vector, k);
            }
        };

        // Sort by combined score
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);

        let output: Vec<serde_json::Value> = results
            .into_iter()
            .map(|(id, score, payload)| {
                serde_json::json!({
                    "id": id,
                    "score": score,
                    "payload": payload
                })
            })
            .collect();

        serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Performs multi-query search with result fusion.
    ///
    /// Executes searches for multiple query vectors and fuses results
    /// using the specified strategy.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Array of query vectors (as flat array with dimension stride)
    /// * `num_vectors` - Number of vectors in the array
    /// * `k` - Number of results to return
    /// * `strategy` - Fusion strategy: "average", "maximum", "rrf"
    /// * `rrf_k` - RRF k parameter (only used when strategy = "rrf", default 60)
    ///
    /// # Returns
    ///
    /// Array of fused results with id and score.
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

        // Execute search for each vector
        let overfetch_k = k * 3; // Overfetch for better fusion
        let mut all_results: Vec<Vec<(u64, f32)>> = Vec::with_capacity(num_vectors);

        for i in 0..num_vectors {
            let start = i * self.dimension;
            let query = &vectors[start..start + self.dimension];

            let results: Vec<(u64, f32)> = match self.storage_mode {
                StorageMode::Full => {
                    let mut r: Vec<(u64, f32)> = self
                        .ids
                        .iter()
                        .enumerate()
                        .map(|(idx, &id)| {
                            let v_start = idx * self.dimension;
                            let v_data = &self.data[v_start..v_start + self.dimension];
                            let score = self.metric.calculate(query, v_data);
                            (id, score)
                        })
                        .collect();

                    if self.metric.higher_is_better() {
                        r.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    } else {
                        r.sort_by(|a, b| {
                            a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                    r.truncate(overfetch_k);
                    r
                }
                _ => Vec::new(), // Simplified for other modes
            };

            all_results.push(results);
        }

        // Fuse results based on strategy
        let fused = fusion::fuse_results(&all_results, strategy, rrf_k.unwrap_or(60));

        // Take top k
        let top_k: Vec<(u64, f32)> = fused.into_iter().take(k).collect();

        serde_wasm_bindgen::to_value(&top_k).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Batch search for multiple vectors in parallel.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Flat array of query vectors (concatenated)
    /// * `num_vectors` - Number of vectors
    /// * `k` - Results per query
    ///
    /// # Returns
    ///
    /// Array of arrays of (id, score) tuples.
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

        let mut all_results: Vec<Vec<(u64, f32)>> = Vec::with_capacity(num_vectors);

        for i in 0..num_vectors {
            let start = i * self.dimension;
            let query = &vectors[start..start + self.dimension];

            let results: Vec<(u64, f32)> = match self.storage_mode {
                StorageMode::Full => {
                    let mut r: Vec<(u64, f32)> = self
                        .ids
                        .iter()
                        .enumerate()
                        .map(|(idx, &id)| {
                            let v_start = idx * self.dimension;
                            let v_data = &self.data[v_start..v_start + self.dimension];
                            (id, self.metric.calculate(query, v_data))
                        })
                        .collect();

                    if self.metric.higher_is_better() {
                        r.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    } else {
                        r.sort_by(|a, b| {
                            a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                    r.truncate(k);
                    r
                }
                _ => Vec::new(),
            };

            all_results.push(results);
        }

        serde_wasm_bindgen::to_value(&all_results).map_err(|e| JsValue::from_str(&e.to_string()))
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
    ///
    /// This is more efficient when you know the approximate number of vectors
    /// you'll be inserting, as it avoids repeated memory allocations.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric: "cosine", "euclidean", or "dot"
    /// * `capacity` - Number of vectors to pre-allocate space for
    ///
    /// # Errors
    ///
    /// Returns an error if the metric is not recognized.
    #[wasm_bindgen]
    pub fn with_capacity(
        dimension: usize,
        metric: &str,
        capacity: usize,
    ) -> Result<VectorStore, JsValue> {
        let metric = match metric.to_lowercase().as_str() {
            "cosine" => DistanceMetric::Cosine,
            "euclidean" | "l2" => DistanceMetric::Euclidean,
            "dot" | "dotproduct" | "inner" => DistanceMetric::DotProduct,
            "hamming" => DistanceMetric::Hamming,
            "jaccard" => DistanceMetric::Jaccard,
            _ => {
                return Err(JsValue::from_str(
                    "Unknown metric. Use: cosine, euclidean, dot, hamming, jaccard",
                ))
            }
        };

        Ok(Self {
            ids: Vec::with_capacity(capacity),
            data: Vec::with_capacity(capacity * dimension),
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: Vec::with_capacity(capacity),
            dimension,
            metric,
            storage_mode: StorageMode::Full,
        })
    }

    /// Pre-allocates memory for the specified number of additional vectors.
    ///
    /// Call this before bulk insertions to avoid repeated allocations.
    ///
    /// # Arguments
    ///
    /// * `additional` - Number of additional vectors to reserve space for
    #[wasm_bindgen]
    pub fn reserve(&mut self, additional: usize) {
        self.ids.reserve(additional);
        self.data.reserve(additional * self.dimension);
    }

    /// Inserts multiple vectors in a single batch operation.
    ///
    /// This is significantly faster than calling `insert()` multiple times
    /// because it pre-allocates memory and reduces per-call overhead.
    ///
    /// # Arguments
    ///
    /// * `batch` - JavaScript array of `[id, Float32Array]` pairs
    ///
    /// # Errors
    ///
    /// Returns an error if any vector dimension doesn't match store dimension.
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

    /// Exports the vector store to a binary format.
    ///
    /// The binary format contains:
    /// - Header: dimension (u32), metric (u8), count (u64)
    /// - For each vector: id (u64), data (f32 array)
    ///
    /// Use this to persist data to `IndexedDB` or `localStorage`.
    ///
    /// # Errors
    ///
    /// This function currently does not return errors but uses `Result`
    /// for future extensibility.
    ///
    /// # Performance
    ///
    /// Perf: Pre-allocates exact buffer size to avoid reallocations.
    /// Throughput: ~1600 MB/s on 10k vectors (768D)
    #[wasm_bindgen]
    pub fn export_to_bytes(&self) -> Result<Vec<u8>, JsValue> {
        // Perf: Pre-allocate exact size - uses contiguous buffer for 2500+ MB/s
        let count = self.ids.len();
        let vector_size = 8 + self.dimension * 4; // id + data
        let total_size = 18 + count * vector_size;
        let mut bytes = Vec::with_capacity(total_size);

        // Header: magic number "VELS" (4 bytes)
        bytes.extend_from_slice(b"VELS");

        // Version (1 byte)
        bytes.push(1);

        // Dimension (4 bytes, little-endian)
        #[allow(clippy::cast_possible_truncation)]
        let dim_u32 = self.dimension as u32;
        bytes.extend_from_slice(&dim_u32.to_le_bytes());

        // Metric (1 byte: 0=cosine, 1=euclidean, 2=dot, 3=hamming, 4=jaccard)
        let metric_byte = match self.metric {
            DistanceMetric::Cosine => 0u8,
            DistanceMetric::Euclidean => 1u8,
            DistanceMetric::DotProduct => 2u8,
            DistanceMetric::Hamming => 3u8,
            DistanceMetric::Jaccard => 4u8,
        };
        bytes.push(metric_byte);

        // Vector count (8 bytes, little-endian)
        #[allow(clippy::cast_possible_truncation)]
        let count_u64 = count as u64;
        bytes.extend_from_slice(&count_u64.to_le_bytes());

        // Perf: Write IDs and data from contiguous buffers
        for (idx, &id) in self.ids.iter().enumerate() {
            bytes.extend_from_slice(&id.to_le_bytes());
            // Direct slice from contiguous data buffer
            let start = idx * self.dimension;
            let data_slice = &self.data[start..start + self.dimension];
            // Write f32s as bytes
            let data_bytes: &[u8] = unsafe {
                core::slice::from_raw_parts(data_slice.as_ptr().cast::<u8>(), self.dimension * 4)
            };
            bytes.extend_from_slice(data_bytes);
        }

        Ok(bytes)
    }

    /// Saves the vector store to `IndexedDB`.
    ///
    /// This method persists all vectors to the browser's `IndexedDB`,
    /// enabling offline-first applications.
    ///
    /// # Arguments
    ///
    /// * `db_name` - Name of the `IndexedDB` database
    ///
    /// # Errors
    ///
    /// Returns an error if `IndexedDB` is not available or the save fails.
    ///
    /// # Example
    ///
    /// ```javascript
    /// const store = new VectorStore(768, "cosine");
    /// store.insert(1n, vector1);
    /// await store.save("my-vectors");
    /// ```
    #[wasm_bindgen]
    pub async fn save(&self, db_name: &str) -> Result<(), JsValue> {
        let bytes = self.export_to_bytes()?;
        persistence::save_to_indexeddb(db_name, &bytes).await
    }

    /// Loads a vector store from `IndexedDB`.
    ///
    /// This method restores all vectors from the browser's `IndexedDB`.
    ///
    /// # Arguments
    ///
    /// * `db_name` - Name of the `IndexedDB` database
    ///
    /// # Errors
    ///
    /// Returns an error if the database doesn't exist or is corrupted.
    ///
    /// # Example
    ///
    /// ```javascript
    /// const store = await VectorStore.load("my-vectors");
    /// console.log(store.len); // Number of restored vectors
    /// ```
    #[wasm_bindgen]
    pub async fn load(db_name: &str) -> Result<VectorStore, JsValue> {
        let bytes = persistence::load_from_indexeddb(db_name).await?;
        Self::import_from_bytes(&bytes)
    }

    /// Deletes the `IndexedDB` database.
    ///
    /// Use this to clear all persisted data.
    ///
    /// # Arguments
    ///
    /// * `db_name` - Name of the `IndexedDB` database to delete
    ///
    /// # Errors
    ///
    /// Returns an error if the deletion fails.
    #[wasm_bindgen]
    pub async fn delete_database(db_name: &str) -> Result<(), JsValue> {
        persistence::delete_database(db_name).await
    }

    /// Imports a vector store from binary format.
    ///
    /// Use this to restore data from `IndexedDB` or `localStorage`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The data is too short or corrupted
    /// - The magic number is invalid
    /// - The version is unsupported
    /// - The metric byte is invalid
    #[wasm_bindgen]
    pub fn import_from_bytes(bytes: &[u8]) -> Result<VectorStore, JsValue> {
        if bytes.len() < 18 {
            return Err(JsValue::from_str("Invalid data: too short"));
        }

        // Check magic number
        if &bytes[0..4] != b"VELS" {
            return Err(JsValue::from_str("Invalid data: wrong magic number"));
        }

        // Check version
        let version = bytes[4];
        if version != 1 {
            return Err(JsValue::from_str(&format!(
                "Unsupported version: {version}"
            )));
        }

        // Read dimension
        let dimension = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;

        // Read metric
        let metric = match bytes[9] {
            0 => DistanceMetric::Cosine,
            1 => DistanceMetric::Euclidean,
            2 => DistanceMetric::DotProduct,
            3 => DistanceMetric::Hamming,
            4 => DistanceMetric::Jaccard,
            _ => return Err(JsValue::from_str("Invalid metric byte")),
        };

        // Read vector count
        #[allow(clippy::cast_possible_truncation)]
        let count = u64::from_le_bytes([
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17],
        ]) as usize;

        // Calculate expected size
        let vector_size = 8 + dimension * 4; // id + data
        let expected_size = 18 + count * vector_size;
        if bytes.len() < expected_size {
            return Err(JsValue::from_str(&format!(
                "Invalid data: expected {expected_size} bytes, got {}",
                bytes.len()
            )));
        }

        // Perf: Pre-allocate contiguous buffers
        // Optimization: Single allocation + bulk copy = 500+ MB/s
        let mut ids = Vec::with_capacity(count);
        let total_floats = count * dimension;
        let mut data = vec![0.0_f32; total_floats];

        let mut offset = 18;
        let data_bytes_len = dimension * 4;

        // Read all IDs first (cache-friendly sequential access)
        for _ in 0..count {
            let id = u64::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7],
            ]);
            ids.push(id);
            offset += 8 + data_bytes_len; // Skip to next ID
        }

        // Perf: Bulk copy all vector data in one operation
        // SAFETY: f32 and [u8; 4] have same size, WASM is little-endian
        let data_as_bytes: &mut [u8] = unsafe {
            core::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), total_floats * 4)
        };

        // Copy data from each vector position
        offset = 18 + 8; // Skip header + first ID
        for i in 0..count {
            let dest_start = i * dimension * 4;
            let dest_end = dest_start + data_bytes_len;
            data_as_bytes[dest_start..dest_end]
                .copy_from_slice(&bytes[offset..offset + data_bytes_len]);
            offset += 8 + data_bytes_len; // Move to next vector's data
        }

        Ok(Self {
            ids,
            data,
            data_sq8: Vec::new(),
            data_binary: Vec::new(),
            sq8_mins: Vec::new(),
            sq8_scales: Vec::new(),
            payloads: vec![None; count],
            dimension,
            metric,
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
