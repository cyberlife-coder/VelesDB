//! HTTP handlers for VelesDB REST API.
//!
//! This module organizes handlers by domain:
//! - `health`: Health check endpoints
//! - `collections`: Collection CRUD operations
//! - `points`: Vector point operations
//! - `search`: Vector similarity search
//! - `query`: VelesQL query execution
//! - `indexes`: Property index management (EPIC-009)
//! - `graph`: Graph operations (EPIC-016/US-031)
//! - `metrics`: Prometheus metrics (EPIC-016/US-034)

pub mod collections;
pub mod graph;
pub mod health;
pub mod indexes;
pub mod metrics;
pub mod points;
pub mod query;
pub mod search;

pub use collections::{create_collection, delete_collection, get_collection, list_collections};
pub use health::health_check;
pub use indexes::{create_index, delete_index, list_indexes};
pub use points::{delete_point, get_point, upsert_points};
pub use query::query;
pub use search::{batch_search, hybrid_search, search, text_search};

// Graph and metrics handlers (EPIC-016) - will be used when routes are added
#[allow(unused_imports)]
pub use graph::{add_edge, get_edges, GraphService};
#[allow(unused_imports)]
pub use metrics::{health_metrics, prometheus_metrics};
