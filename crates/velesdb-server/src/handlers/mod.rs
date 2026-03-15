//! HTTP handlers for VelesDB REST API.
//!
//! This module organizes handlers by domain:
//! - `health`: Health check endpoints
//! - `collections`: Collection CRUD operations
//! - `admin`: Stats, config, guardrails, and analyze endpoints
//! - `points`: Vector point operations
//! - `search`: Vector similarity search
//! - `query`: VelesQL query execution
//! - `indexes`: Property index management (EPIC-009)
//! - `graph`: Graph operations (EPIC-016/US-031)
//! - `metrics`: Prometheus metrics (requires `prometheus` feature)

pub mod admin;
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

pub use admin::{
    analyze_collection, get_collection_config, get_collection_stats, get_guardrails,
    update_guardrails,
};
pub use collections::{
    collection_sanity, create_collection, delete_collection, flush_collection, get_collection,
    is_empty, list_collections,
};
pub use health::health_check;
pub use indexes::{create_index, delete_index, list_indexes};
pub use points::{delete_point, get_point, stream_insert, stream_upsert_points, upsert_points};
// EPIC-058 US-007: match_query handler for /collections/{name}/match
pub use match_query::match_query;
pub use query::{aggregate, explain, query};
pub use search::{
    batch_search, hybrid_search, multi_query_search, search, search_ids, text_search,
};

// Graph handlers (EPIC-016) - exported via lib.rs
#[allow(unused_imports)]
pub use graph::{
    add_edge, get_edges, get_node_degree, traverse_graph, DegreeResponse, TraversalResultItem,
    TraversalStats, TraverseRequest, TraverseResponse,
};

// Metrics handlers - conditional on prometheus feature
#[cfg(feature = "prometheus")]
#[allow(unused_imports)]
pub use metrics::{health_metrics, prometheus_metrics};
