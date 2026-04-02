//! Graph CLI commands for VelesDB.
//!
//! Provides CLI commands for graph operations using direct core calls.
//! All commands work offline without a running server.
//!
//! Handler implementations live in [`crate::graph_handlers`].
//! Display helpers shared with the REPL live in [`crate::graph_display`].

use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::graph_handlers;

/// Traversal algorithm selection.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum TraverseAlgo {
    #[default]
    Bfs,
    Dfs,
}

/// Edge direction for neighbor queries.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum Direction {
    #[default]
    Out,
    In,
    Both,
}

/// Graph subcommands
#[derive(Subcommand)]
pub enum GraphAction {
    /// Add an edge to the graph
    AddEdge {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Edge ID
        id: u64,
        /// Source node ID
        source: u64,
        /// Target node ID
        target: u64,
        /// Edge label (relationship type)
        label: String,
    },

    /// List edges, optionally filtered by label
    GetEdges {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Filter by edge label
        #[arg(long)]
        label: Option<String>,
        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Get the degree of a node
    Degree {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Node ID
        node_id: u64,
        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Traverse the graph using BFS or DFS
    Traverse {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Source node ID
        source: u64,
        /// Traversal algorithm (bfs, dfs)
        #[arg(long, value_enum, default_value = "bfs")]
        algorithm: TraverseAlgo,
        /// Maximum depth
        #[arg(short = 'd', long, default_value = "3")]
        max_depth: u32,
        /// Maximum number of results
        #[arg(short = 'l', long, default_value = "100")]
        limit: usize,
        /// Filter by relationship types (comma-separated)
        #[arg(short = 'r', long)]
        rel_types: Option<String>,
        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Get neighbors of a node (incoming, outgoing, or both)
    Neighbors {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Node ID
        node_id: u64,
        /// Edge direction (in, out, both)
        #[arg(long, value_enum, default_value = "out")]
        direction: Direction,
        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Store a JSON payload on a graph node
    StorePayload {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Node ID
        node_id: u64,
        /// JSON payload (e.g., '{"name": "Alice"}')
        payload: String,
    },

    /// Retrieve the JSON payload of a graph node
    GetPayload {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Node ID
        node_id: u64,
    },

    /// Remove an edge by ID
    RemoveEdge {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Edge ID to remove
        edge_id: u64,
    },

    /// Count total edges in the graph
    Count {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Search graph nodes by embedding similarity
    Search {
        /// Path to database directory
        path: PathBuf,
        /// Collection name
        collection: String,
        /// Query vector as JSON array (e.g., '[0.1, 0.2, 0.3]')
        vector: String,
        /// Number of results to return
        #[arg(short = 'k', long, default_value = "10")]
        top_k: usize,
        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// List all nodes with stored payloads (paginated)
    Nodes {
        /// Path to database directory
        path: PathBuf,
        /// Graph collection name
        collection: String,
        /// Page number (1-indexed)
        #[arg(short, long, default_value = "1")]
        page: usize,
        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },
}

/// Handle graph subcommands with direct core calls.
///
/// # Errors
///
/// Returns an error if the database cannot be opened or the graph collection
/// is not found.
pub fn handle(action: GraphAction) -> anyhow::Result<()> {
    match action {
        GraphAction::AddEdge {
            path,
            collection,
            id,
            source,
            target,
            label,
        } => graph_handlers::handle_add_edge(&path, &collection, id, source, target, &label),
        GraphAction::GetEdges {
            path,
            collection,
            label,
            format,
        } => graph_handlers::handle_get_edges(&path, &collection, label.as_deref(), &format),
        GraphAction::Degree {
            path,
            collection,
            node_id,
            format,
        } => graph_handlers::handle_degree(&path, &collection, node_id, &format),
        GraphAction::Traverse {
            path,
            collection,
            source,
            algorithm,
            max_depth,
            limit,
            rel_types,
            format,
        } => graph_handlers::handle_traverse(
            &path,
            &collection,
            source,
            algorithm,
            max_depth,
            limit,
            rel_types.as_deref(),
            &format,
        ),
        GraphAction::Neighbors {
            path,
            collection,
            node_id,
            direction,
            format,
        } => graph_handlers::handle_neighbors(&path, &collection, node_id, direction, &format),
        GraphAction::StorePayload {
            path,
            collection,
            node_id,
            payload,
        } => graph_handlers::handle_store_payload(&path, &collection, node_id, &payload),
        GraphAction::GetPayload {
            path,
            collection,
            node_id,
        } => graph_handlers::handle_get_payload(&path, &collection, node_id),
        GraphAction::RemoveEdge {
            path,
            collection,
            edge_id,
        } => graph_handlers::handle_remove_edge(&path, &collection, edge_id),
        GraphAction::Count {
            path,
            collection,
            format,
        } => graph_handlers::handle_count(&path, &collection, &format),
        GraphAction::Search {
            path,
            collection,
            vector,
            top_k,
            format,
        } => graph_handlers::handle_graph_search(&path, &collection, &vector, top_k, &format),
        GraphAction::Nodes {
            path,
            collection,
            page,
            format,
        } => graph_handlers::handle_graph_nodes(&path, &collection, page, &format),
    }
}

#[cfg(test)]
#[path = "graph_bdd_tests.rs"]
mod graph_bdd_tests;
