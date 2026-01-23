//! Graph CLI commands for VelesDB.
//!
//! Provides CLI commands for graph operations.
//! Note: Graph operations require a running VelesDB server.

use clap::Subcommand;
use colored::Colorize;
use std::path::PathBuf;

/// Graph subcommands
#[derive(Subcommand)]
pub enum GraphAction {
    /// Traverse the graph using BFS or DFS
    Traverse {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Source node ID
        source: u64,

        /// Traversal strategy (bfs, dfs)
        #[arg(short, long, default_value = "bfs")]
        strategy: String,

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
}

/// Handle graph subcommands (EPIC-016 US-050)
/// Note: Graph operations require a running VelesDB server.
pub fn handle(action: GraphAction) {
    // Graph operations are only available via REST API
    // The server manages EdgeStore in memory
    println!(
        "{} Graph operations require a running VelesDB server.",
        "ℹ️".cyan()
    );
    println!("  Start server: velesdb-server --data-dir ./data");
    println!("  Then use curl or the TypeScript SDK to interact with graph endpoints:\n");

    match action {
        GraphAction::Traverse {
            path: _,
            collection,
            source,
            strategy,
            max_depth,
            limit,
            rel_types,
            format: _,
        } => {
            let rel_types_json = rel_types
                .map(|s| {
                    let types: Vec<&str> = s.split(',').map(str::trim).collect();
                    serde_json::json!(types)
                })
                .unwrap_or(serde_json::json!([]));

            println!(
                "  curl -X POST http://localhost:8080/collections/{}/graph/traverse \\",
                collection
            );
            println!("    -H 'Content-Type: application/json' \\");
            println!(
                "    -d '{}'",
                serde_json::json!({
                    "source": source,
                    "strategy": strategy,
                    "max_depth": max_depth,
                    "limit": limit,
                    "rel_types": rel_types_json
                })
            );
        }
        GraphAction::Degree {
            path: _,
            collection,
            node_id,
            format: _,
        } => {
            println!(
                "  curl http://localhost:8080/collections/{}/graph/nodes/{}/degree",
                collection, node_id
            );
        }
        GraphAction::AddEdge {
            path: _,
            collection,
            id,
            source,
            target,
            label,
        } => {
            println!(
                "  curl -X POST http://localhost:8080/collections/{}/graph/edges \\",
                collection
            );
            println!("    -H 'Content-Type: application/json' \\");
            println!(
                "    -d '{}'",
                serde_json::json!({
                    "id": id,
                    "source": source,
                    "target": target,
                    "label": label,
                    "properties": {}
                })
            );
        }
    }

    println!();
    println!(
        "{} For persistent graph storage, use the Python SDK with Collection.add_edge().",
        "💡".yellow()
    );
}
