//! # `VelesDB` Core
//!
//! High-performance vector database engine written in Rust.
//!
//! `VelesDB` is a local-first vector database designed for semantic search,
//! recommendation systems, and RAG (Retrieval-Augmented Generation) applications.
//!
//! ## Features
//!
//! - **Blazing Fast**: HNSW index with explicit SIMD (4x faster)
//! - **5 Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
//! - **Hybrid Search**: Vector + BM25 full-text with RRF fusion
//! - **Quantization**: SQ8 (4x) and Binary (32x) memory compression
//! - **Persistent Storage**: Memory-mapped files for efficient disk access
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use velesdb_core::{Database, DistanceMetric, Point, StorageMode};
//! use serde_json::json;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a new database
//!     let db = Database::open("./data")?;
//!
//!     // Create a collection (all 5 metrics available)
//!     db.create_collection("documents", 768, DistanceMetric::Cosine)?;
//!     // Or with quantization: DistanceMetric::Hamming + StorageMode::Binary
//!
//!     let collection = db.get_collection("documents").ok_or("Collection not found")?;
//!
//!     // Insert vectors (upsert takes ownership)
//!     collection.upsert(vec![
//!         Point::new(1, vec![0.1; 768], Some(json!({"title": "Hello World"}))),
//!     ])?;
//!
//!     // Search for similar vectors
//!     let query_vector = vec![0.1; 768];
//!     let results = collection.search(&query_vector, 10)?;
//!
//!     // Hybrid search (vector + text)
//!     let hybrid = collection.hybrid_search(&query_vector, "hello", 5, Some(0.7))?;
//!     # Ok(())
//! }
//! ```

#![warn(missing_docs)]
// Clippy lints configured in workspace Cargo.toml [workspace.lints.clippy]
#![cfg_attr(
    test,
    allow(
        clippy::large_stack_arrays,
        clippy::doc_markdown,
        clippy::uninlined_format_args,
        clippy::single_match_else,
        clippy::cast_lossless,
        clippy::manual_assert
    )
)]

#[cfg(feature = "persistence")]
pub mod agent;
pub mod alloc_guard;
#[cfg(test)]
mod alloc_guard_tests;
pub mod cache;
#[cfg(feature = "persistence")]
pub mod collection;
#[cfg(feature = "persistence")]
pub mod column_store;
#[cfg(all(test, feature = "persistence"))]
mod column_store_tests;
pub mod compression;
pub mod config;
#[cfg(test)]
mod config_tests;
pub mod distance;
#[cfg(test)]
mod distance_tests;
pub mod error;
#[cfg(test)]
mod error_tests;
pub mod filter;
#[cfg(test)]
mod filter_like_tests;
#[cfg(test)]
mod filter_tests;
pub mod fusion;
pub mod gpu;
#[cfg(test)]
mod gpu_tests;
#[cfg(feature = "persistence")]
pub mod guardrails;
#[cfg(all(test, feature = "persistence"))]
mod guardrails_tests;
pub mod half_precision;
#[cfg(test)]
mod half_precision_tests;
#[cfg(feature = "persistence")]
pub mod index;
pub mod metrics;
#[cfg(test)]
mod metrics_tests;
pub mod perf_optimizations;
#[cfg(test)]
mod perf_optimizations_tests;
pub mod point;
#[cfg(test)]
mod point_tests;
pub mod quantization;
#[cfg(test)]
mod quantization_tests;
pub mod simd_dispatch;
#[cfg(test)]
mod simd_dispatch_tests;
#[cfg(test)]
mod simd_epic073_tests;
/// Sparse vector types, inverted index, and search -- always compiled (no persistence dependency).
pub mod sparse_index;
// simd_explicit removed - consolidated into simd_native (EPIC-075)
pub mod simd_native;
#[cfg(test)]
mod simd_native_tests;
#[cfg(target_arch = "aarch64")]
pub mod simd_neon;
#[cfg(target_arch = "aarch64")]
pub mod simd_neon_prefetch;
// simd_ops removed - direct dispatch via simd_native (EPIC-CLEANUP)
#[cfg(test)]
mod simd_prefetch_x86_tests;
#[cfg(test)]
mod simd_tests;
#[cfg(feature = "persistence")]
pub mod storage;
pub mod sync;
#[cfg(all(not(target_arch = "wasm32"), feature = "update-check"))]
pub mod update_check;
pub mod vector_ref;
#[cfg(test)]
mod vector_ref_tests;
pub mod velesql;

#[cfg(all(not(target_arch = "wasm32"), feature = "update-check"))]
pub use update_check::{check_for_updates, spawn_update_check};
#[cfg(all(not(target_arch = "wasm32"), feature = "update-check"))]
pub use update_check::{compute_instance_hash, UpdateCheckConfig};

#[cfg(feature = "persistence")]
pub use index::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

#[cfg(feature = "persistence")]
pub use collection::streaming::BackpressureError;
#[cfg(feature = "persistence")]
pub use collection::{
    // Collection: internal executor kept pub for backward compat and internal modules.
    // Suppression de l'export = PR dédiée (requiert ~40 corrections de références internes).
    Collection,
    // Public user-facing types — 3 typed collections replace Collection as primary API
    CollectionType,
    // Graph API types (user-visible)
    EdgeType,
    GraphCollection,
    GraphEdge,
    GraphNode,
    GraphSchema,
    IndexInfo,
    MetadataCollection,
    NodeType,
    TraversalResult,
    ValueType,
    VectorCollection,
};
pub use distance::DistanceMetric;
pub use error::{Error, Result};
pub use filter::{Condition, Filter};
pub use point::{Point, SearchResult};
pub use quantization::{
    cosine_similarity_quantized, cosine_similarity_quantized_simd, dot_product_quantized,
    dot_product_quantized_simd, euclidean_squared_quantized, euclidean_squared_quantized_simd,
    BinaryQuantizedVector, QuantizedVector, StorageMode,
};

#[cfg(feature = "persistence")]
pub use column_store::{
    BatchUpdate, BatchUpdateResult, BatchUpsertResult, ColumnStore, ColumnStoreError, ColumnType,
    ColumnValue, ExpireResult, StringId, StringTable, TypedColumn, UpsertResult,
};
pub use config::{
    ConfigError, HnswConfig, LimitsConfig, QuantizationConfig, QuantizationType, SearchConfig,
    SearchMode, VelesConfig,
};
#[cfg(feature = "persistence")]
pub use config::{LoggingConfig, ServerConfig, StorageConfig};
pub use fusion::{FusionError, FusionStrategy};
pub use metrics::{
    average_metrics, compute_latency_percentiles, hit_rate, mean_average_precision, mrr, ndcg_at_k,
    precision_at_k, recall_at_k, LatencyStats,
};

#[cfg(feature = "persistence")]
mod database;
#[cfg(feature = "persistence")]
pub mod observer;

#[cfg(feature = "persistence")]
pub use database::Database;
#[cfg(feature = "persistence")]
pub use observer::DatabaseObserver;
