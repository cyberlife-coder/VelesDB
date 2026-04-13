//! Native HNSW implementation.
//!
//! Custom HNSW implementation optimized for VelesDB use cases.
//! Goal: Remove dependency on external hnsw_rs library.
//!
//! # Status
//!
//! Active native implementation used by the HNSW index internals.
//! This module is performance-critical and continuously optimized.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │           NativeHnsw<D>                 │
//! │  D: DistanceEngine (CPU/GPU/SIMD)      │
//! ├─────────────────────────────────────────┤
//! │  layers: Vec<Layer>                     │
//! │  entry_point: AtomicUsize (NO_ENTRY_POINT sentinel)│
//! │  params: HnswParams                     │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # References
//!
//! - Paper: "Efficient and robust approximate nearest neighbor search
//!   using Hierarchical Navigable Small World graphs" (Malkov & Yashunin, 2016)
//! - arXiv: <https://arxiv.org/abs/1603.09320>

// Low-level HNSW internals: precision-loss casts are validated, doc_markdown
// flags false positives on algorithm names (HNSW, RaBitQ, VAMANA).
#![allow(clippy::doc_markdown)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

mod backend_adapter;
mod batch_schedule;
pub(crate) mod columnar_distance;
pub(crate) mod columnar_vectors;
mod distance;
mod dual_precision;
mod graph;
mod graph_io;
mod int8_traversal;
pub(crate) mod layer;
mod ordered_float;
mod quantization;
pub(crate) mod rabitq_precision;
mod rabitq_traversal;
mod search;

pub use backend_adapter::{NativeHnswBackend, NativeNeighbour};
pub use distance::{CachedSimdDistance, CpuDistance, DistanceEngine};
pub use dual_precision::{DualPrecisionConfig, DualPrecisionHnsw};
pub use graph::{NativeHnsw, DEFAULT_ALPHA, NO_ENTRY_POINT};
pub use layer::{Layer, NodeId};
pub use quantization::{QuantizedVector, QuantizedVectorStore, ScalarQuantizer};
pub use rabitq_precision::{RaBitQPrecisionConfig, RaBitQPrecisionHnsw};
pub use search::SearchResult;

#[cfg(test)]
mod backend_adapter_tests;
#[cfg(test)]
mod columnar_vectors_tests;
#[cfg(test)]
mod distance_tests;
#[cfg(test)]
mod dual_precision_tests;
#[cfg(test)]
mod graph_tests;
#[cfg(test)]
mod layer_tests;
#[cfg(test)]
mod ordered_float_tests;
#[cfg(test)]
mod quantization_tests;
#[cfg(test)]
mod rabitq_precision_tests;
#[cfg(test)]
mod search_pipeline_tests;
#[cfg(test)]
mod tests;
