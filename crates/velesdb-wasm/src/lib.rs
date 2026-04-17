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
//! - **Multiple metrics**: Cosine, Euclidean, Dot Product
//! - **Half-precision**: f16/bf16 support for 50% memory reduction
//!
//! Distance kernels run on the scalar code paths of `velesdb-core` under
//! `wasm32-unknown-unknown`. Explicit WASM SIMD128 intrinsics are not wired
//! in this crate; the `wasm-opt` post-processing stage may emit SIMD128
//! opportunistically when safe, but that is an optimizer choice and is not
//! part of the public API contract.
//!
//! # Usage (JavaScript)
//!
//! ```javascript
//! import init, { VectorStore } from 'velesdb-wasm';
//!
//! await init();
//!
//! const store = new VectorStore(768, "cosine");
//! store.insert(1n, new Float32Array([0.1, 0.2, ...]));
//!
//! // search() returns Array<[bigint, number]>
//! const results = store.search(new Float32Array([0.1, ...]), 10);
//! // results = [[1n, 0.95], [42n, 0.87], ...]
//! ```

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

mod agent;
mod database;
mod filter;
mod fusion;
mod graph;
mod graph_persistence;
mod graph_worker_hints;
mod hybrid_quantized;
mod idb_helpers;
mod parsing;
mod persistence;
mod serialization;
pub mod sparse;
mod store_get;
mod store_insert;
mod store_new;
mod store_search;
mod text_search;
mod vector_ops;
mod vector_store;
mod vector_store_persistence;
mod velesql;
mod velesql_helpers;

pub use agent::SemanticMemory;
pub use database::{WasmCollectionHandle, WasmDatabase};
pub use graph::{GraphEdge, GraphNode, GraphStore};
pub use vector_store::VectorStore;
pub use velesdb_core::DistanceMetric;

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
    /// Product Quantization — **WASM limitation**: PQ requires `rayon`/`persistence`
    /// which are unavailable in WASM. This variant uses the SQ8 codepath as a
    /// fallback. For true PQ, use the native `velesdb-core` crate.
    ProductQuantization,
    /// `RaBitQ`: 1-bit with rotation + scalar correction (32x compression).
    /// **WASM limitation**: training requires `ndarray`/`persistence`. Falls back
    /// to Full precision in WASM builds.
    RaBitQ,
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
