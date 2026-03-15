//! Core Collection implementation (Lifecycle & CRUD).
//!
//! This module provides the main implementation of Collection:
//! - Lifecycle: create, open, flush, save
//! - CRUD: upsert, get, delete
//! - Index management: `create_property_index`, `create_range_index`,
//!   `list_indexes`, `drop_index`

mod crud;
mod crud_helpers;
#[cfg(test)]
mod crud_tests;
mod graph_api;
#[cfg(test)]
mod graph_api_tests;
mod index_management;
#[cfg(test)]
mod index_management_tests;
mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;
mod statistics;

pub use index_management::IndexInfo;
pub use lifecycle::{MAX_DIMENSION, MIN_DIMENSION};

// All implementations are in submodules, no re-exports needed here
// as they extend the Collection type defined in types.rs
