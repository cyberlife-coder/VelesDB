//! Graph CLI command handler implementations.
//!
//! Each function handles one `GraphAction` variant. Extracted from `graph.rs`
//! to keep file sizes under 500 NLOC per code-quality rules.

use colored::Colorize;
use std::path::PathBuf;
use velesdb_core::collection::graph::TraversalConfig;
use velesdb_core::GraphEdge;

use crate::graph::{Direction, TraverseAlgo};
use crate::graph_display;
use crate::helpers;

/// Open a graph collection from a database path.
pub(crate) fn open_graph(
    path: &PathBuf,
    collection: &str,
) -> anyhow::Result<velesdb_core::GraphCollection> {
    let db = velesdb_core::Database::open(path)?;
    db.get_graph_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Graph collection '{}' not found", collection))
}

/// Converts a slice of edges to a JSON array value.
fn edges_to_json_value(edges: &[GraphEdge]) -> serde_json::Value {
    let data: Vec<_> = edges.iter().map(graph_display::edge_to_json).collect();
    serde_json::Value::Array(data)
}

pub(crate) fn handle_add_edge(
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

pub(crate) fn handle_get_edges(
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

pub(crate) fn handle_degree(
    path: &PathBuf,
    collection: &str,
    node_id: u64,
    format: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let (in_deg, out_deg) = col.node_degree(node_id);
    if format == "json" {
        helpers::print_json(&serde_json::json!({
            "node_id": node_id, "in_degree": in_deg,
            "out_degree": out_deg, "total_degree": in_deg + out_deg
        }))?;
    } else {
        graph_display::print_degree(node_id, in_deg, out_deg);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_traverse(
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
            .map(
                |r| serde_json::json!({"target_id": r.target_id, "depth": r.depth, "path": r.path}),
            )
            .collect();
        helpers::print_json(&serde_json::Value::Array(data))?;
    } else {
        graph_display::print_traversal(&results, algo_label, source, max_depth);
    }
    Ok(())
}

pub(crate) fn handle_neighbors(
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
            dir_label.green()
        );
        graph_display::print_edge_list(&edges, "No neighbors found.");
    }
    Ok(())
}

pub(crate) fn handle_store_payload(
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
        node_id.to_string().green()
    );
    Ok(())
}

pub(crate) fn handle_get_payload(
    path: &PathBuf,
    collection: &str,
    node_id: u64,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    match col
        .get_node_payload(node_id)
        .map_err(|e| anyhow::anyhow!("{e}"))?
    {
        Some(val) => println!("{}", serde_json::to_string_pretty(&val)?),
        None => println!("null"),
    }
    Ok(())
}

pub(crate) fn handle_remove_edge(
    path: &PathBuf,
    collection: &str,
    edge_id: u64,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    if col.remove_edge(edge_id) {
        col.flush()
            .map_err(|e| anyhow::anyhow!("Flush failed: {e}"))?;
        println!(
            "{} Edge {} removed",
            "✅".green(),
            edge_id.to_string().green()
        );
    } else {
        println!(
            "{} Edge {} not found",
            "⚠️".yellow(),
            edge_id.to_string().yellow()
        );
    }
    Ok(())
}

pub(crate) fn handle_count(path: &PathBuf, collection: &str, format: &str) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    let count = col.edge_count();
    let node_count = col.all_node_ids().len();
    if format == "json" {
        helpers::print_json(&serde_json::json!({"edge_count": count, "node_count": node_count}))?;
    } else {
        println!(
            "\n{} '{}'\n",
            "Graph Stats".bold().underline(),
            collection.green()
        );
        println!("  {} {}", "Edges:".cyan(), count);
        println!("  {} {}", "Nodes:".cyan(), node_count);
        println!();
    }
    Ok(())
}

pub(crate) fn handle_graph_search(
    path: &PathBuf,
    collection: &str,
    vector_json: &str,
    top_k: usize,
    format: &str,
) -> anyhow::Result<()> {
    let col = open_graph(path, collection)?;
    if !col.has_embeddings() {
        anyhow::bail!(
            "Graph collection '{}' has no embeddings. Create with embeddings to enable search.",
            collection
        );
    }
    let query: Vec<f32> = serde_json::from_str(vector_json)
        .map_err(|e| anyhow::anyhow!("Invalid vector JSON: {e}"))?;
    let results = col
        .search_by_embedding(&query, top_k)
        .map_err(|e| anyhow::anyhow!("Search failed: {e}"))?;
    if format == "json" {
        let data: Vec<_> = results.iter()
            .map(|r| serde_json::json!({"id": r.point.id, "score": r.score, "payload": r.point.payload}))
            .collect();
        helpers::print_json(&serde_json::Value::Array(data))?;
    } else if results.is_empty() {
        println!("No results found.\n");
    } else {
        println!(
            "\n{} ({} results)\n",
            "Graph Search Results".bold().underline(),
            results.len()
        );
        for r in &results {
            println!(
                "  id={} score={:.6}",
                r.point.id.to_string().cyan(),
                r.score
            );
        }
        println!();
    }
    Ok(())
}

pub(crate) fn handle_graph_nodes(
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
            .map(|(id, payload)| serde_json::json!({"id": id, "payload": payload}))
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
            node_page.total_edges
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
                payload_str
            );
        }
        println!(
            "\n  Total: {} node(s) on this page\n",
            node_page.entries.len()
        );
    }
}
