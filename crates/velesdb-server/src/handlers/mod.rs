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
//! - `metrics`: Prometheus metrics (requires `prometheus` feature)

pub mod collections;
pub mod graph;
pub mod health;
pub mod indexes;
pub mod match_query;
pub mod points;
pub mod query;
pub mod search;

#[cfg(feature = "prometheus")]
pub mod metrics;

pub use collections::{create_collection, delete_collection, get_collection, list_collections};
pub use health::health_check;
pub use indexes::{create_index, delete_index, list_indexes};
pub use points::{delete_point, get_point, upsert_points};
// EPIC-058 US-007: match_query handler for /collections/{name}/match
pub use match_query::match_query;
pub use query::{explain, query};
pub use search::{batch_search, hybrid_search, multi_query_search, search, text_search};

// Graph handlers (EPIC-016) - exported via lib.rs
#[allow(unused_imports)]
pub use graph::{
    add_edge, get_edges, get_node_degree, traverse_graph, DegreeResponse, GraphService,
    TraversalResultItem, TraversalStats, TraverseRequest, TraverseResponse,
};

// Metrics handlers - conditional on prometheus feature
#[cfg(feature = "prometheus")]
#[allow(unused_imports)]
pub use metrics::{health_metrics, prometheus_metrics};
