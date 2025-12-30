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
mod persistence;
mod simd;

pub use distance::DistanceMetric;

/// A vector store for in-memory vector search.
///
/// # Performance
///
/// Uses contiguous memory layout for optimal cache locality and fast
/// serialization. Vector data is stored in a single buffer rather than
/// individual Vec allocations.
#[wasm_bindgen]
pub struct VectorStore {
    /// Vector IDs in insertion order
    ids: Vec<u64>,
    /// Contiguous buffer: all vector data packed sequentially
    data: Vec<f32>,
    dimension: usize,
    metric: DistanceMetric,
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
            dimension,
            metric,
        })
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
            self.ids.remove(idx);
            let start = idx * self.dimension;
            self.data.drain(start..start + self.dimension);
        }

        // Append to contiguous buffer
        self.ids.push(id);
        self.data.extend_from_slice(vector);

        Ok(())
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

        // Perf: Iterate over contiguous buffer with better cache locality
        let mut results: Vec<(u64, f32)> = self
            .ids
            .iter()
            .enumerate()
            .map(|(idx, &id)| {
                let start = idx * self.dimension;
                let v_data = &self.data[start..start + self.dimension];
                let score = self.metric.calculate(query, v_data);
                (id, score)
            })
            .collect();

        // Sort by relevance
        if self.metric.higher_is_better() {
            results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        results.truncate(k);

        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Removes a vector by ID.
    #[wasm_bindgen]
    pub fn remove(&mut self, id: u64) -> bool {
        if let Some(idx) = self.ids.iter().position(|&x| x == id) {
            self.ids.remove(idx);
            let start = idx * self.dimension;
            self.data.drain(start..start + self.dimension);
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
    }

    /// Returns memory usage estimate in bytes.
    #[wasm_bindgen]
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.ids.len() * std::mem::size_of::<u64>() + self.data.len() * 4
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
            _ => {
                return Err(JsValue::from_str(
                    "Unknown metric. Use: cosine, euclidean, dot",
                ))
            }
        };

        Ok(Self {
            ids: Vec::with_capacity(capacity),
            data: Vec::with_capacity(capacity * dimension),
            dimension,
            metric,
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
            dimension,
            metric,
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

// Tests for VectorStore are in distance.rs and simd.rs modules
// The wasm-bindgen VectorStore tests require wasm-bindgen-test and must
// be run with `wasm-pack test --node`
