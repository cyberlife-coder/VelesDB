//! Core Collection implementation (Lifecycle & CRUD).
//!
//! This module provides the main implementation of Collection:
//! - Lifecycle: create, open, flush, save
//! - CRUD: upsert, get, delete
//! - Index management: `create_property_index`, `create_range_index`,
//!   `list_indexes`, `drop_index`

mod advanced_config;
#[cfg(all(test, feature = "persistence"))]
mod advanced_config_tests;
#[cfg(feature = "persistence")]
mod bm25_coverage;
#[cfg(all(test, feature = "persistence"))]
mod bm25_coverage_tests;
#[cfg(all(test, feature = "persistence"))]
mod bm25_durability_tests;
#[cfg(all(test, feature = "persistence"))]
mod bm25_flush_race_tests;
mod bulk_import;
mod crud;
mod crud_bulk;
#[cfg(all(test, feature = "persistence"))]
mod crud_bulk_tests;
mod crud_helpers;
mod crud_histogram;
mod crud_indexing;
mod crud_read_delete;
#[cfg(all(test, feature = "persistence"))]
mod crud_read_delete_tests;
#[cfg(test)]
mod crud_tests;
mod flush;
#[cfg(all(test, feature = "persistence"))]
mod flush_defer_tests;
mod graph_api;
mod graph_api_node_payload;
#[cfg(test)]
mod graph_api_tests;
#[cfg(all(test, feature = "persistence"))]
mod graph_edge_wal_recovery_tests;
mod graph_nodes;
mod graph_property_index_wiring;
mod index_management;
#[cfg(test)]
mod index_management_tests;
mod lifecycle;
mod lifecycle_create;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(all(test, feature = "persistence"))]
mod metadata_rows_tests;
#[cfg(all(test, feature = "persistence"))]
mod open_reload_tests;
#[cfg(feature = "persistence")]
mod quantizer_restore;
#[cfg(all(test, feature = "persistence"))]
mod quantizer_restore_tests;
#[cfg(all(test, feature = "persistence"))]
mod rayon_caller_tests;
mod recovery;
#[cfg(all(test, feature = "persistence"))]
mod recovery_tests;
mod rows;
#[cfg(all(test, feature = "persistence"))]
mod rows_tests;
mod scroll;
#[cfg(all(test, feature = "persistence"))]
mod scroll_tests;
mod size;
#[cfg(all(test, feature = "persistence"))]
mod size_tests;
mod sparse_lifecycle;
mod statistics;
#[cfg(feature = "persistence")]
mod streaming_ops;
#[cfg(all(test, feature = "persistence"))]
mod ttl_read_tests;
#[cfg(all(test, feature = "persistence"))]
mod vector_rows_tests;
#[cfg(test)]
pub(crate) mod write_hooks;

pub use crate::validation::{MAX_DIMENSION, MIN_DIMENSION};
pub use index_management::IndexInfo;
pub use scroll::ScrollBatch;
pub(crate) use size::NodeLocks;

// All implementations are in submodules, no re-exports needed here
// as they extend the Collection type defined in types.rs
