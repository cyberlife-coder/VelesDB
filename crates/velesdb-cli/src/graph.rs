//! Graph CLI commands for VelesDB.
//!
//! Provides CLI commands for graph operations using direct core calls.
//! All commands work offline without a running server.
//!
//! Display helpers shared with the REPL live in [`crate::graph_display`].
//! JSON serialization uses [`crate::helpers::print_json`] to avoid
//! duplicating `serde_json::to_string_pretty` + `println!` calls.

use clap::{Subcommand, ValueEnum};
use colored::Colorize;
use std::path::PathBuf;
use velesdb_core::collection::graph::TraversalConfig;
use velesdb_core::GraphEdge;

use crate::graph_display;
use crate::helpers;

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
        } => handle_add_edge(&path, &collection, id, source, target, &label),

        GraphAction::GetEdges {
            path,
            collection,
            label,
            format,
        } => handle_get_edges(&path, &collection, label.as_deref(), &format),

        GraphAction::Degree {
            path,
            collection,
            node_id,
            format,
        } => handle_degree(&path, &collection, node_id, &format),

        GraphAction::Traverse {
            path,
            collection,
            source,
            algorithm,
            max_depth,
            limit,
            rel_types,
            format,
        } => handle_traverse(
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
        } => handle_neighbors(&path, &collection, node_id, direction, &format),

        GraphAction::StorePayload {
            path,
            collection,
            node_id,
            payload,
        } => handle_store_payload(&path, &collection, node_id, &payload),

        GraphAction::GetPayload {
            path,
            collection,
            node_id,
        } => handle_get_payload(&path, &collection, node_id),

        GraphAction::Nodes {
            path,
            collection,
            page,
            format,
        } => handle_graph_nodes(&path, &collection, page, &format),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn open_graph(path: &PathBuf, collection: &str) -> anyhow::Result<velesdb_core::GraphCollection> {
    let db = velesdb_core::Database::open(path)?;
    db.get_graph_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Graph collection '{}' not found", collection))
}

/// Converts a slice of edges to a JSON array value.
fn edges_to_json_value(edges: &[GraphEdge]) -> serde_json::Value {
    let data: Vec<_> = edges.iter().map(graph_display::edge_to_json).collect();
    serde_json::Value::Array(data)
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

fn handle_add_edge(
    path: &PathBuf,
    collection: &str,
    id: u64,
    source: u64,
    target: u64,
    label: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let edge = GraphEdge::new(id, source, target, label).map_err(|e| anyhow::anyhow!("{e}"))?;
    col.add_edge(edge).map_err(|e| anyhow::anyhow!("{e}"))?;
    col.flush()
        .map_err(|e| anyhow::anyhow!("Flush failed: {e}"))?;

    println!(
        "{} Edge {} added: {} --[{}]--> {}",
        "✅".green(),
        id.to_string().green(),
        source,
        label.cyan(),
        target,
    );
    Ok(())
}

fn handle_get_edges(
    path: &PathBuf,
    collection: &str,
    label: Option<&str>,
    format: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let edges = col.get_edges(label);

    if format == "json" {
        helpers::print_json(&edges_to_json_value(&edges))?;
    } else {
        let filter_msg = label.map_or_else(String::new, |l| format!(" (label={})", l.cyan()));
        println!("\n{}{}\n", "Edges".bold().underline(), filter_msg);
        graph_display::print_edge_list(&edges, "No edges found.");
    }
    Ok(())
}

fn handle_degree(
    path: &PathBuf,
    collection: &str,
    node_id: u64,
    format: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let (in_deg, out_deg) = col.node_degree(node_id);

    if format == "json" {
        helpers::print_json(&serde_json::json!({
            "node_id": node_id,
            "in_degree": in_deg,
            "out_degree": out_deg,
            "total_degree": in_deg + out_deg
        }))?;
    } else {
        graph_display::print_degree(node_id, in_deg, out_deg);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_traverse(
    path: &PathBuf,
    collection: &str,
    source: u64,
    algorithm: TraverseAlgo,
    max_depth: u32,
    limit: usize,
    rel_types: Option<&str>,
    format: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;

    let rel_vec: Vec<String> = rel_types
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let config = TraversalConfig {
        min_depth: 1,
        max_depth,
        limit,
        rel_types: rel_vec,
    };

    let algo_label = match algorithm {
        TraverseAlgo::Bfs => "BFS",
        TraverseAlgo::Dfs => "DFS",
    };

    let results = match algorithm {
        TraverseAlgo::Bfs => col.traverse_bfs(source, &config),
        TraverseAlgo::Dfs => col.traverse_dfs(source, &config),
    };

    if format == "json" {
        let data: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "target_id": r.target_id,
                    "depth": r.depth,
                    "path": r.path
                })
            })
            .collect();
        helpers::print_json(&serde_json::Value::Array(data))?;
    } else {
        graph_display::print_traversal(&results, algo_label, source, max_depth);
    }
    Ok(())
}

fn handle_neighbors(
    path: &PathBuf,
    collection: &str,
    node_id: u64,
    direction: Direction,
    format: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;

    let edges = match direction {
        Direction::Out => col.get_outgoing(node_id),
        Direction::In => col.get_incoming(node_id),
        Direction::Both => {
            let mut all = col.get_outgoing(node_id);
            all.extend(col.get_incoming(node_id));
            all
        }
    };

    let dir_label = match direction {
        Direction::Out => "outgoing",
        Direction::In => "incoming",
        Direction::Both => "all",
    };

    if format == "json" {
        helpers::print_json(&edges_to_json_value(&edges))?;
    } else {
        println!(
            "\n{} (node={}, direction={})\n",
            "Neighbors".bold().underline(),
            node_id,
            dir_label.green(),
        );
        graph_display::print_edge_list(&edges, "No neighbors found.");
    }
    Ok(())
}

fn handle_store_payload(
    path: &PathBuf,
    collection: &str,
    node_id: u64,
    payload_str: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let payload: serde_json::Value = serde_json::from_str(payload_str)
        .map_err(|e| anyhow::anyhow!("Invalid JSON payload: {e}"))?;
    col.upsert_node_payload(node_id, &payload)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    col.flush()
        .map_err(|e| anyhow::anyhow!("Flush failed: {e}"))?;

    println!(
        "{} Payload stored on node {}",
        "✅".green(),
        node_id.to_string().green(),
    );
    Ok(())
}

fn handle_get_payload(path: &PathBuf, collection: &str, node_id: u64) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let payload = col
        .get_node_payload(node_id)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    match payload {
        Some(val) => println!("{}", serde_json::to_string_pretty(&val)?),
        None => println!("null"),
    }
    Ok(())
}

fn handle_graph_nodes(
    path: &PathBuf,
    collection: &str,
    page: usize,
    format: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let node_page = graph_display::paginate_graph_nodes(&col, page, 20)?;

    if format == "json" {
        let data: Vec<serde_json::Value> = node_page
            .entries
            .iter()
            .map(|(id, payload)| {
                serde_json::json!({
                    "id": id,
                    "payload": payload
                })
            })
            .collect();
        helpers::print_json(&serde_json::Value::Array(data))?;
    } else {
        println!(
            "\n{} in '{}' -- Page {}/{} ({} unique nodes from {} edges)\n",
            "Nodes".bold().underline(),
            collection.green(),
            node_page.page,
            node_page.total_pages.max(1),
            node_page.total_nodes,
            node_page.total_edges,
        );
        print_node_page_table(&node_page);
    }
    Ok(())
}

/// Print paginated node data in table format.
fn print_node_page_table(node_page: &graph_display::NodePage) {
    if node_page.entries.is_empty() {
        println!("  No nodes on this page.\n");
    } else {
        for (node_id, payload) in &node_page.entries {
            let payload_str = match payload {
                Some(v) => serde_json::to_string(v).unwrap_or_default(),
                None => "null".to_string(),
            };
            println!(
                "  {} {} payload={}",
                format!("[{}]", node_id).cyan(),
                node_id.to_string().green(),
                payload_str,
            );
        }
        println!(
            "\n  Total: {} node(s) on this page\n",
            node_page.entries.len()
        );
    }
}
