//! VelesDB WASM - Vector search in the browser
//!
//! This crate provides WebAssembly bindings for VelesDB's core vector operations.
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
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Returns true if the store is empty.
    #[wasm_bindgen(getter)]
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Returns the vector dimension.
    #[wasm_bindgen(getter)]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Inserts a vector with the given ID.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the vector
    /// * `vector` - Float32Array of the vector data
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
    /// * `query` - Query vector as Float32Array
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Array of [id, score] pairs sorted by relevance.
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
    pub fn memory_usage(&self) -> usize {
        self.vectors.len() * (std::mem::size_of::<u64>() + self.dimension * 4)
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
