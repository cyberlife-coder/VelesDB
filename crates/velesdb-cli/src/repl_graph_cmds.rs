//! REPL commands for graph operations.
//!
//! Covers: `.graph` (dispatcher), `.graph add-edge`, `.graph edges`,
//! `.graph degree`, `.graph traverse`, `.graph neighbors`,
//! `.graph remove-edge`, `.graph count`, `.graph search`,
//! `.graph store-payload`, `.graph get-payload`, `.graph nodes`.

use colored::Colorize;
use velesdb_core::collection::graph::TraversalConfig;
use velesdb_core::Database;

use crate::graph_display;
use crate::repl_commands::{parse_flag, CommandResult};

/// Parse a node/source ID from command parts, returning a `CommandResult::Error` on failure.
fn parse_node_id(parts: &[&str], idx: usize) -> Result<u64, CommandResult> {
    parts[idx]
        .parse()
        .map_err(|_| CommandResult::Error(format!("Invalid node ID: {}", parts[idx])))
}

pub(crate) fn cmd_graph(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        print_graph_help();
        return CommandResult::Continue;
    }

    let sub = parts[1];
    match sub {
        "add-edge" => cmd_graph_add_edge(db, parts),
        "edges" => cmd_graph_edges(db, parts),
        "degree" => cmd_graph_degree(db, parts),
        "traverse" => cmd_graph_traverse(db, parts),
        "neighbors" => cmd_graph_neighbors(db, parts),
        "remove-edge" => cmd_graph_remove_edge(db, parts),
        "count" => cmd_graph_count(db, parts),
        "search" => cmd_graph_search(db, parts),
        "store-payload" => cmd_graph_store_payload(db, parts),
        "get-payload" => cmd_graph_get_payload(db, parts),
        "nodes" => cmd_graph_nodes(db, parts),
        "help" => {
            print_graph_help();
            CommandResult::Continue
        }
        _ => CommandResult::Error(format!("Unknown graph command: {sub}. Use '.graph help'.")),
    }
}

fn resolve_graph_collection(
    db: &Database,
    parts: &[&str],
    expected_idx: usize,
) -> Result<velesdb_core::GraphCollection, CommandResult> {
    let name = match parts.get(expected_idx) {
        Some(n) => *n,
        None => return Err(CommandResult::Error("Missing collection name.".to_string())),
    };
    db.get_graph_collection(name)
        .ok_or_else(|| CommandResult::Error(format!("Graph collection '{name}' not found")))
}

fn cmd_graph_add_edge(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 7 {
        println!("Usage: .graph add-edge <collection> <id> <source> <target> <label>\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let id: u64 = match parts[3].parse() {
        Ok(v) => v,
        Err(_) => return CommandResult::Error(format!("Invalid edge ID: {}", parts[3])),
    };
    let source: u64 = match parts[4].parse() {
        Ok(v) => v,
        Err(_) => return CommandResult::Error(format!("Invalid source ID: {}", parts[4])),
    };
    let target: u64 = match parts[5].parse() {
        Ok(v) => v,
        Err(_) => return CommandResult::Error(format!("Invalid target ID: {}", parts[5])),
    };
    let label = parts[6];

    let edge = match velesdb_core::GraphEdge::new(id, source, target, label) {
        Ok(e) => e,
        Err(e) => return CommandResult::Error(format!("{e}")),
    };
    if let Err(e) = col.add_edge(edge) {
        return CommandResult::Error(format!("{e}"));
    }

    println!(
        "{} Edge {} added: {} --[{}]--> {}",
        "\u{2705}".green(),
        id.to_string().green(),
        source,
        label.cyan(),
        target,
    );
    CommandResult::Continue
}

fn cmd_graph_edges(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 3 {
        println!("Usage: .graph edges <collection> [--label <label>]\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let label = parse_flag(parts, "--label");
    let edges = col.get_edges(label.as_deref());
    graph_display::print_edge_list(&edges, "No edges found.");
    CommandResult::Continue
}

fn cmd_graph_degree(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .graph degree <collection> <node_id>\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let node_id: u64 = match parse_node_id(parts, 3) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let (in_deg, out_deg) = col.node_degree(node_id);
    graph_display::print_degree(node_id, in_deg, out_deg);
    CommandResult::Continue
}

fn cmd_graph_traverse(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .graph traverse <collection> <source> [--algo bfs|dfs] [--depth N] [--limit N] [--rel-types X,Y]\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let source: u64 = match parse_node_id(parts, 3) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let algo = parse_flag(parts, "--algo").unwrap_or_else(|| "bfs".to_string());
    let max_depth: u32 = parse_flag(parts, "--depth")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let limit: usize = parse_flag(parts, "--limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let rel_types: Vec<String> = parse_flag(parts, "--rel-types")
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let config = TraversalConfig {
        min_depth: 1,
        max_depth,
        limit,
        rel_types,
    };

    let algo_label = match algo.as_str() {
        "dfs" => "DFS",
        _ => "BFS",
    };

    let results = match algo.as_str() {
        "dfs" => col.traverse_dfs(source, &config),
        _ => col.traverse_bfs(source, &config),
    };

    graph_display::print_traversal(&results, algo_label, source, max_depth);
    CommandResult::Continue
}

fn cmd_graph_neighbors(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .graph neighbors <collection> <node_id> [--direction in|out|both]\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let node_id: u64 = match parse_node_id(parts, 3) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let dir = parse_flag(parts, "--direction").unwrap_or_else(|| "out".to_string());

    let edges = match dir.as_str() {
        "in" => col.get_incoming(node_id),
        "both" => {
            let mut all = col.get_outgoing(node_id);
            all.extend(col.get_incoming(node_id));
            all
        }
        _ => col.get_outgoing(node_id),
    };

    println!(
        "\n{} (node={}, direction={})\n",
        "Neighbors".bold().underline(),
        node_id,
        dir.green(),
    );
    graph_display::print_edge_list(&edges, "No neighbors found.");
    CommandResult::Continue
}

fn cmd_graph_remove_edge(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .graph remove-edge <collection> <edge_id>\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let edge_id: u64 = match parts[3].parse() {
        Ok(v) => v,
        Err(_) => return CommandResult::Error(format!("Invalid edge ID: {}", parts[3])),
    };

    if col.remove_edge(edge_id) {
        println!(
            "{} Edge {} removed",
            "\u{2705}".green(),
            edge_id.to_string().green(),
        );
    } else {
        println!(
            "{} Edge {} not found",
            "\u{26a0}".yellow(),
            edge_id.to_string().yellow(),
        );
    }
    CommandResult::Continue
}

fn cmd_graph_count(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 3 {
        println!("Usage: .graph count <collection>\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let edge_count = col.edge_count();
    let stored_nodes = col.all_node_ids().len();
    let total_points = col.len();

    println!("\n{}\n", "Graph Stats".bold().underline(),);
    println!("  {} {}", "Edges:".cyan(), edge_count);
    println!("  {} {}", "Stored nodes:".cyan(), stored_nodes);
    println!("  {} {}", "Total points:".cyan(), total_points);
    println!();
    CommandResult::Continue
}

fn cmd_graph_search(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .graph search <collection> <vector_json> [k]\n");
        println!(
            "  Example: {} my_graph [0.1,0.2,0.3] 10\n",
            ".graph search".yellow()
        );
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };

    if !col.has_embeddings() {
        return CommandResult::Error(
            "Graph collection has no embeddings. Create with embeddings to enable search."
                .to_string(),
        );
    }

    let vector_json = parts[3];
    let k: usize = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);

    let query: Vec<f32> = match serde_json::from_str(vector_json) {
        Ok(v) => v,
        Err(e) => {
            return CommandResult::Error(format!(
                "Invalid vector JSON: {e}\nExpected format: [0.1, 0.2, ...]"
            ));
        }
    };

    match col.search_by_embedding(&query, k) {
        Ok(results) => {
            if results.is_empty() {
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
        }
        Err(e) => return CommandResult::Error(format!("Search error: {e}")),
    }
    CommandResult::Continue
}

fn cmd_graph_store_payload(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 5 {
        println!("Usage: .graph store-payload <collection> <node_id> <json_payload>\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let node_id: u64 = match parse_node_id(parts, 3) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let payload_str = parts[4..].join(" ");
    let payload: serde_json::Value = match serde_json::from_str(&payload_str) {
        Ok(v) => v,
        Err(e) => return CommandResult::Error(format!("Invalid JSON: {e}")),
    };

    if let Err(e) = col.upsert_node_payload(node_id, &payload) {
        return CommandResult::Error(format!("{e}"));
    }
    let _ = col.flush();

    println!(
        "{} Payload stored on node {}",
        "\u{2705}".green(),
        node_id.to_string().green(),
    );
    CommandResult::Continue
}

fn cmd_graph_get_payload(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .graph get-payload <collection> <node_id>\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let node_id: u64 = match parse_node_id(parts, 3) {
        Ok(v) => v,
        Err(r) => return r,
    };

    match col.get_node_payload(node_id) {
        Ok(Some(val)) => {
            // SAFETY invariant: serde_json::Value always serializes successfully.
            println!(
                "{}",
                serde_json::to_string_pretty(&val)
                    .expect("invariant: serde_json::Value serializes")
            );
        }
        Ok(None) => println!("null"),
        Err(e) => return CommandResult::Error(format!("{e}")),
    }
    CommandResult::Continue
}

fn cmd_graph_nodes(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 3 {
        println!("Usage: .graph nodes <collection> [--page N]\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let page: usize = parse_flag(parts, "--page")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    match graph_display::paginate_graph_nodes(&col, page, 20) {
        Ok(node_page) => {
            println!(
                "\n{} -- Page {}/{} ({} nodes, {} edges)\n",
                "Nodes".bold().underline(),
                node_page.page,
                node_page.total_pages.max(1),
                node_page.total_nodes,
                node_page.total_edges,
            );
            if node_page.entries.is_empty() {
                println!("  No nodes on this page.\n");
            } else {
                for (node_id, payload) in &node_page.entries {
                    let payload_str = match payload {
                        Some(v) => serde_json::to_string(v).unwrap_or_default(),
                        None => "null".to_string(),
                    };
                    println!(
                        "  {} payload={}",
                        format!("[{}]", node_id).cyan(),
                        payload_str,
                    );
                }
                println!();
            }
        }
        Err(e) => return CommandResult::Error(format!("{e}")),
    }
    CommandResult::Continue
}

fn print_graph_help() {
    println!("\n{}", "Graph Commands".bold().underline());
    println!();
    println!(
        "  {} Add an edge",
        ".graph add-edge <col> <id> <src> <tgt> <label>".yellow()
    );
    println!(
        "  {}       List edges",
        ".graph edges <col> [--label X]".yellow()
    );
    println!(
        "  {}         Node degree",
        ".graph degree <col> <node>".yellow()
    );
    println!(
        "  {}   Graph traversal",
        ".graph traverse <col> <src> [--algo bfs|dfs] [--depth N]".yellow()
    );
    println!(
        "  {} Node neighbors",
        ".graph neighbors <col> <node> [--direction in|out|both]".yellow()
    );
    println!(
        "  {}  Remove an edge",
        ".graph remove-edge <col> <edge_id>".yellow()
    );
    println!(
        "  {}          Edge/node count",
        ".graph count <col>".yellow()
    );
    println!(
        "  {} Embedding search",
        ".graph search <col> <vector_json> [k]".yellow()
    );
    println!(
        "  {} Store payload",
        ".graph store-payload <col> <node> <json>".yellow()
    );
    println!(
        "  {}  Get payload",
        ".graph get-payload <col> <node>".yellow()
    );
    println!("  {} List nodes", ".graph nodes <col> [--page N]".yellow());
    println!();
}

#[cfg(test)]
#[path = "repl_graph_bdd_tests.rs"]
mod repl_graph_bdd_tests;
