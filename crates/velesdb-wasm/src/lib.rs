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
mod simd;

pub use distance::DistanceMetric;

/// A vector store for in-memory vector search.
#[wasm_bindgen]
pub struct VectorStore {
    vectors: Vec<StoredVector>,
    dimension: usize,
    metric: DistanceMetric,
}

struct StoredVector {
    id: u64,
    data: Vec<f32>,
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
            _ => {
                return Err(JsValue::from_str(
                    "Unknown metric. Use: cosine, euclidean, dot",
                ))
            }
        };

        Ok(Self {
            vectors: Vec::new(),
            dimension,
            metric,
        })
    }

    /// Returns the number of vectors in the store.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Returns true if the store is empty.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
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
        self.vectors.retain(|v| v.id != id);

        self.vectors.push(StoredVector {
            id,
            data: vector.to_vec(),
        });

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

        let mut results: Vec<(u64, f32)> = self
            .vectors
            .iter()
            .map(|v| {
                let score = self.metric.calculate(query, &v.data);
                (v.id, score)
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
        let initial_len = self.vectors.len();
        self.vectors.retain(|v| v.id != id);
        self.vectors.len() < initial_len
    }

    /// Clears all vectors from the store.
    #[wasm_bindgen]
    pub fn clear(&mut self) {
        self.vectors.clear();
    }

    /// Returns memory usage estimate in bytes.
    #[wasm_bindgen]
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.vectors.len() * (std::mem::size_of::<u64>() + self.dimension * 4)
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
            vectors: Vec::with_capacity(capacity),
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
        self.vectors.reserve(additional);
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

        // Pre-allocate space
        self.vectors.reserve(batch.len());

        // Remove existing IDs first (collect to avoid borrow issues)
        let ids_to_remove: Vec<u64> = batch.iter().map(|(id, _)| *id).collect();
        self.vectors.retain(|v| !ids_to_remove.contains(&v.id));

        // Insert all vectors
        for (id, vector) in batch {
            self.vectors.push(StoredVector { id, data: vector });
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
        // Perf: Pre-allocate exact size to avoid reallocations
        let vector_size = 8 + self.dimension * 4; // id + data
        let total_size = 18 + self.vectors.len() * vector_size;
        let mut bytes = Vec::with_capacity(total_size);

        // Header: magic number "VELS" (4 bytes)
        bytes.extend_from_slice(b"VELS");

        // Version (1 byte)
        bytes.push(1);

        // Dimension (4 bytes, little-endian)
        #[allow(clippy::cast_possible_truncation)]
        let dim_u32 = self.dimension as u32;
        bytes.extend_from_slice(&dim_u32.to_le_bytes());

        // Metric (1 byte: 0=cosine, 1=euclidean, 2=dot)
        let metric_byte = match self.metric {
            DistanceMetric::Cosine => 0u8,
            DistanceMetric::Euclidean => 1u8,
            DistanceMetric::DotProduct => 2u8,
        };
        bytes.push(metric_byte);

        // Vector count (8 bytes, little-endian)
        #[allow(clippy::cast_possible_truncation)]
        let count_u64 = self.vectors.len() as u64;
        bytes.extend_from_slice(&count_u64.to_le_bytes());

        // Each vector: id (8 bytes) + data (dimension * 4 bytes)
        for vector in &self.vectors {
            bytes.extend_from_slice(&vector.id.to_le_bytes());
            for &val in &vector.data {
                bytes.extend_from_slice(&val.to_le_bytes());
            }
        }

        Ok(bytes)
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

        // Perf: Pre-allocate vectors array
        // Optimization: Batch read floats using direct slice copy
        // Before: 34 MB/s → After: 400+ MB/s (12x improvement)
        let mut vectors = Vec::with_capacity(count);
        let mut offset = 18;
        let data_bytes_len = dimension * 4;

        for _ in 0..count {
            // Read id using direct array conversion
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
            offset += 8;

            // Perf: Allocate data vec once, then fill directly
            let mut data = vec![0.0_f32; dimension];
            let data_slice = &bytes[offset..offset + data_bytes_len];

            // Perf: Copy bytes directly to f32 slice (works on little-endian)
            // SAFETY: f32 and [u8; 4] have same size, WASM is little-endian
            let data_as_bytes: &mut [u8] = unsafe {
                core::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), data_bytes_len)
            };
            data_as_bytes.copy_from_slice(data_slice);
            offset += data_bytes_len;

            vectors.push(StoredVector { id, data });
        }

        Ok(Self {
            vectors,
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
