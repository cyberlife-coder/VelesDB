//! HTTP handlers for VelesDB REST API.
//!
//! This module organizes handlers by domain:
//! - `health`: Health check endpoints
//! - `collections`: Collection CRUD operations
//! - `points`: Vector point operations
//! - `search`: Vector similarity search
//! - `query`: VelesQL query execution
//! - `indexes`: Property index management (EPIC-009)

pub mod collections;
pub mod health;
pub mod indexes;
pub mod points;
pub mod query;
pub mod search;

pub use collections::{create_collection, delete_collection, get_collection, list_collections};
pub use health::health_check;
pub use indexes::{create_index, delete_index, list_indexes};
pub use points::{delete_point, get_point, upsert_points};
pub use query::query;
pub use search::{batch_search, hybrid_search, search, text_search};
