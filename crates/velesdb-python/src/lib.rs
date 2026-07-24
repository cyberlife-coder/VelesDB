// Python SDK - pedantic/nursery lints relaxed for PyO3 FFI boundary
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
#![allow(clippy::useless_conversion)]
//! Python bindings for `VelesDB` vector database.
//!
//! This module provides a Pythonic interface to VelesDB using PyO3.
//!
//! # Example
//!
//! ```python
//! import velesdb
//!
//! # Open database
//! db = velesdb.Database("./my_data")
//!
//! # Create collection
//! collection = db.create_collection("documents", dimension=768, metric="cosine")
//!
//! # Insert vectors
//! collection.upsert([
//!     {"id": 1, "vector": [0.1, 0.2, ...], "payload": {"title": "Doc 1"}}
//! ])
//!
//! # Search (canonical API; collection.search(...) is deprecated since v1.15)
//! results = collection.search_request(SearchOptions(vector=[0.1, 0.2, ...], top_k=10))
//! ```

mod agent;
mod agent_memory_service;
mod collection;
mod collection_helpers;
mod database;
mod database_stats;
mod exceptions;
mod fusion;
mod graph;
mod graph_collection;
mod graph_collection_query;
mod graph_store;
mod observer;
mod options;
mod streaming_config;
mod streaming_runtime;
mod utils;
mod velesql;
mod velesql_helpers;

pub use collection::search_options::SearchOptions;
pub use collection::Collection;
pub use database::Database;
pub use fusion::FusionStrategy;
pub use graph::{dict_to_edge, dict_to_node, edge_to_dict, node_to_dict, traversal_to_dict};
pub use graph_collection::{PyGraphCollection, PyGraphSchema};
pub use graph_store::{GraphStore, StreamingConfig, TraversalResult};
pub use options::{
    AutoReindexOptions, HnswConfigOptions, HnswOptions, LimitsOptions, QuantizationOptions,
    SearchConfigOptions, StorageOptions, VelesConfigOptions,
};
pub use streaming_config::StreamingIngestConfig;

use pyo3::prelude::*;
use pyo3::types::PyTuple;

/// Search result from a vector query.
#[pyclass(frozen)]
pub struct SearchResult {
    #[pyo3(get)]
    id: u64,
    #[pyo3(get)]
    score: f32,
    #[pyo3(get)]
    payload: Py<PyAny>,
}

/// VelesDB - A high-performance vector database for AI applications.
///
/// Example:
///     >>> import velesdb
///     >>> db = velesdb.Database("./my_data")
///     >>> collection = db.create_collection("docs", dimension=768)
///     >>> collection.upsert([{"id": 1, "vector": [...], "payload": {"title": "Doc"}}])
///     >>> results = collection.search_request(SearchOptions(vector=[...], top_k=10))
#[pymodule]
fn velesdb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Database>()?;
    m.add_class::<Collection>()?;
    m.add_class::<SearchResult>()?;
    m.add_class::<FusionStrategy>()?;

    // SearchOptions builder (issue #717 v1.15 additive path)
    m.add_class::<SearchOptions>()?;

    // Scroll iterator (issue #429)
    m.add_class::<collection::scroll::ScrollIterator>()?;

    // Persistent graph collection (Phase 1)
    m.add_class::<PyGraphCollection>()?;
    m.add_class::<PyGraphSchema>()?;

    // In-memory graph classes (EPIC-016/US-030, US-032)
    m.add_class::<GraphStore>()?;
    m.add_class::<StreamingConfig>()?;
    m.add_class::<TraversalResult>()?;

    // Streaming-ingestion config (STREAM-1)
    m.add_class::<StreamingIngestConfig>()?;

    // Agent memory classes (EPIC-010/US-005)
    m.add_class::<agent::AgentMemory>()?;
    m.add_class::<agent::PySemanticMemory>()?;
    m.add_class::<agent::PyEpisodicMemory>()?;
    m.add_class::<agent::PyProceduralMemory>()?;
    // High-level memory wedge (remember/recall/relate/forget/why + extraction).
    m.add_class::<agent_memory_service::PyMemoryService>()?;

    // VelesQL query language classes (EPIC-056/US-001)
    velesql::register_velesql_module(m)?;

    // Typed options dataclasses (Wave 3 Commit 10)
    options::register_options(m)?;

    // Typed exception hierarchy (issue #427)
    exceptions::register_exceptions(m)?;

    // Canonical enum name sets, single-sourced from velesdb-core so the
    // bindings and integrations never drift from the engine's variants.
    // Exposed as immutable tuples of `str`.
    m.add(
        "DISTANCE_METRICS",
        PyTuple::new(m.py(), velesdb_core::DISTANCE_METRIC_NAMES)?,
    )?;
    m.add(
        "STORAGE_MODES",
        PyTuple::new(m.py(), velesdb_core::STORAGE_MODE_NAMES)?,
    )?;
    m.add(
        "CONDITION_TYPES",
        PyTuple::new(m.py(), velesdb_core::CONDITION_TYPE_NAMES)?,
    )?;

    // Add version info
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    Ok(())
}
