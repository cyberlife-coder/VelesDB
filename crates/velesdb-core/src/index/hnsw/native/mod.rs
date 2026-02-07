//! Native HNSW Implementation - Prototype
//!
//! Custom HNSW implementation optimized for VelesDB use cases.
//! Goal: Remove dependency on external hnsw_rs library.
//!
//! # Status
//!
//! **PROTOTYPE** - Not yet integrated into production code.
//! This module demonstrates the architecture for a custom HNSW implementation.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │           NativeHnsw<D>                 │
//! │  D: DistanceEngine (CPU/GPU/SIMD)      │
//! ├─────────────────────────────────────────┤
//! │  layers: Vec<Layer>                     │
//! │  entry_point: Option<NodeId>            │
//! │  params: HnswParams                     │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # References
//!
//! - Paper: "Efficient and robust approximate nearest neighbor search
//!   using Hierarchical Navigable Small World graphs" (Malkov & Yashunin, 2016)
//! - arXiv: <https://arxiv.org/abs/1603.09320>

// Prototype code - suppress warnings until production integration
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::unused_self)]

mod backend_adapter;
mod distance;
mod dual_precision;
mod graph;
pub(crate) mod layer;
mod ordered_float;
mod quantization;
mod search;

pub use backend_adapter::{NativeHnswBackend, NativeNeighbour};
pub use distance::{
    CachedSimdDistance, CpuDistance, DistanceEngine, NativeSimdDistance, SimdDistance,
};
pub use dual_precision::{DualPrecisionConfig, DualPrecisionHnsw};
pub use graph::NativeHnsw;
pub use layer::{Layer, NodeId};
pub use quantization::{QuantizedVector, QuantizedVectorStore, ScalarQuantizer};
pub use search::SearchResult;

#[cfg(test)]
mod backend_adapter_tests;
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
mod tests;
