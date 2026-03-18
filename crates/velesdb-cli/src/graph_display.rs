//! Shared display helpers for graph CLI output.
//!
//! Consolidates formatting patterns that were duplicated across `graph.rs`
//! (standalone CLI commands) and `repl_commands.rs` (REPL `.graph` commands).

use colored::Colorize;
use std::collections::BTreeSet;
use velesdb_core::{GraphCollection, GraphEdge};

/// Convert a [`GraphEdge`] to its JSON representation.
#[must_use]
pub fn edge_to_json(edge: &GraphEdge) -> serde_json::Value {
    serde_json::json!({
        "id": edge.id(),
        "source": edge.source(),
        "target": edge.target(),
        "label": edge.label(),
        "properties": edge.properties()
    })
}

/// Print a single edge in the standard `[id] source --[label]--> target` format.
///
/// When `show_props` is true, appends `props=<json>` to display edge properties.
pub fn print_edge_line(edge: &GraphEdge, show_props: bool) {
    if show_props {
        println!(
            "  {} {} --[{}]--> {}  props={}",
            format!("[{}]", edge.id()).cyan(),
            edge.source(),
            edge.label().green(),
            edge.target(),
            serde_json::to_string(edge.properties()).unwrap_or_default(),
        );
    } else {
        println!(
            "  {} {} --[{}]--> {}",
            format!("[{}]", edge.id()).cyan(),
            edge.source(),
            edge.label().green(),
            edge.target(),
        );
    }
}

/// Print a list of edges to stdout (table format).
///
/// Prints each edge on its own line followed by a total count.
/// If the list is empty, prints a "none found" message instead.
pub fn print_edge_list(edges: &[GraphEdge], empty_msg: &str) {
    if edges.is_empty() {
        println!("  {empty_msg}\n");
    } else {
        for e in edges {
            print_edge_line(e, true);
        }
        println!("\n  Total: {} edge(s)\n", edges.len());
    }
}

/// Print node degree information (table format).
pub fn print_degree(node_id: u64, in_deg: usize, out_deg: usize) {
    println!("\n{}", "Node Degree".bold().underline());
    println!("  {} {}", "Node ID:".cyan(), node_id.to_string().green());
    println!("  {} {}", "In-degree:".cyan(), in_deg);
    println!("  {} {}", "Out-degree:".cyan(), out_deg);
    println!("  {} {}", "Total:".cyan(), in_deg + out_deg);
    println!();
}

/// Print traversal results (table format).
pub fn print_traversal(
    results: &[velesdb_core::collection::graph::TraversalResult],
    algo_label: &str,
    source: u64,
    max_depth: u32,
) {
    println!(
        "\n{} (algorithm={}, source={}, max_depth={})\n",
        "Traversal Results".bold().underline(),
        algo_label.green(),
        source,
        max_depth,
    );
    if results.is_empty() {
        println!("  No results found.\n");
    } else {
        for r in results {
            println!(
                "  depth={} target={} path={:?}",
                r.depth,
                r.target_id.to_string().green(),
                r.path,
            );
        }
        println!("\n  Total: {} result(s)\n", results.len());
    }
}

/// Collect unique node IDs from edges into a sorted set.
#[must_use]
pub fn unique_node_ids(edges: &[GraphEdge]) -> BTreeSet<u64> {
    edges
        .iter()
        .flat_map(|e| [e.source(), e.target()])
        .collect()
}

/// A single page of graph node data.
pub struct NodePage {
    /// Node ID and optional payload for each node on this page.
    pub entries: Vec<(u64, Option<serde_json::Value>)>,
    /// Total number of unique nodes across all pages.
    pub total_nodes: usize,
    /// Total number of edges in the collection.
    pub total_edges: usize,
    /// Current page number (1-indexed).
    pub page: usize,
    /// Total number of pages.
    pub total_pages: usize,
}

/// Paginate graph nodes and fetch their payloads.
///
/// # Errors
///
/// Returns an error if fetching any node payload fails.
pub fn paginate_graph_nodes(
    col: &GraphCollection,
    page: usize,
    page_size: usize,
) -> anyhow::Result<NodePage> {
    let edges = col.get_edges(None);
    let all_node_ids = unique_node_ids(&edges);
    let total_nodes = all_node_ids.len();
    let page = page.max(1);
    let total_pages = total_nodes.div_ceil(page_size);
    let offset = (page - 1) * page_size;

    let page_ids: Vec<u64> = all_node_ids
        .into_iter()
        .skip(offset)
        .take(page_size)
        .collect();

    let mut entries = Vec::with_capacity(page_ids.len());
    for &node_id in &page_ids {
        let payload = col
            .get_node_payload(node_id)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        entries.push((node_id, payload));
    }

    Ok(NodePage {
        entries,
        total_nodes,
        total_edges: edges.len(),
        page,
        total_pages,
    })
}
