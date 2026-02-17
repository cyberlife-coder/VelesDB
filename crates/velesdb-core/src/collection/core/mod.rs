//! Core Collection implementation (Lifecycle & CRUD).
//!
//! This module provides the main implementation of Collection:
//! - Lifecycle: create, open, flush, save
//! - CRUD: upsert, get, delete
//! - Index management: `create_property_index`, `create_range_index`, `list_indexes`, `drop_index`

mod crud;
mod graph_api;
mod index_management;
#[cfg(test)]
mod index_management_tests;
mod lifecycle;
mod statistics;

pub use graph_api::TraversalResult;
pub use index_management::IndexInfo;

// All implementations are in submodules, no re-exports needed here
// as they extend the Collection type defined in types.rs
