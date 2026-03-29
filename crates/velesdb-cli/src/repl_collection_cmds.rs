//! REPL commands for collection inspection and management.
//!
//! Covers: `.collections`, `.schema`, `.describe`, `.count`, `.sample`,
//! `.browse`, `.nodes`, `.stats`.

use colored::Colorize;
use std::collections::HashMap;
use velesdb_core::Database;

use crate::collection_helpers;
use crate::graph_display;
use crate::helpers;
use crate::repl_commands::CommandResult;

pub(crate) fn cmd_collections(db: &Database) -> CommandResult {
    let collections = db.list_collections();
    if collections.is_empty() {
        println!("No collections found.\n");
    } else {
        println!("{}", "Collections:".bold());
        for name in collections {
            println!("  - {}", name.green());
        }
        println!();
    }
    CommandResult::Continue
}

pub(crate) fn cmd_schema(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .schema <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let cfg = col.config();
            println!("{} {}", "Collection:".bold(), cfg.name.green());
            println!("  Type:      Vector");
            println!("  Dimension: {}", cfg.dimension);
            println!("  Metric:    {:?}", cfg.metric);
            println!("  Points:    {}", cfg.point_count);
            println!();
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let edge_count = col.get_edges(None).len();
            println!("{} {}", "Collection:".bold(), col.name().green());
            println!("  Type:      Graph");
            println!("  Edges:     {}", edge_count);
            println!("  Embeddings: {}", col.has_embeddings());
            println!();
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            println!("{} {}", "Collection:".bold(), col.name().green());
            println!("  Type:      Metadata");
            println!("  Items:     {}", col.len());
            println!();
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_describe(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .describe <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let type_label = collection_helpers::collection_type_label(db, name);
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let cfg = col.config();
            println!("\n{}", "Collection Details".bold().underline());
            println!("  {} {}", "Name:".cyan(), cfg.name.green());
            println!("  {} {}", "Type:".cyan(), type_label.green());
            println!("  {} {}", "Dimension:".cyan(), cfg.dimension);
            println!("  {} {:?}", "Metric:".cyan(), cfg.metric);
            println!("  {} {}", "Point Count:".cyan(), cfg.point_count);
            println!("  {} {:?}", "Storage Mode:".cyan(), cfg.storage_mode);
            let vector_size = cfg.dimension * 4;
            let estimated_mb = (cfg.point_count * vector_size) as f64 / 1_000_000.0;
            println!(
                "  {} {:.2} MB (vectors only)",
                "Est. Memory:".cyan(),
                estimated_mb
            );
            println!();
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let edge_count = col.get_edges(None).len();
            println!("\n{}", "Collection Details".bold().underline());
            println!("  {} {}", "Name:".cyan(), col.name().green());
            println!("  {} {}", "Type:".cyan(), type_label.green());
            println!("  {} {}", "Edges:".cyan(), edge_count);
            println!("  {} {}", "Embeddings:".cyan(), col.has_embeddings());
            println!("  {} {:?}", "Schema:".cyan(), col.schema());
            println!();
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            println!("\n{}", "Collection Details".bold().underline());
            println!("  {} {}", "Name:".cyan(), col.name().green());
            println!("  {} {}", "Type:".cyan(), type_label.green());
            println!("  {} {}", "Item Count:".cyan(), col.len());
            println!();
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_count(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .count <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let count = col.len();
            println!("Count: {} records\n", count.to_string().green());
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let count = col.get_edges(None).len();
            println!("Count: {} edges\n", count.to_string().green());
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            let count = col.len();
            println!("Count: {} items\n", count.to_string().green());
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_sample(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .sample <collection_name> [count]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let count: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);

    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let all_ids = col.all_ids();
            let sample_ids: Vec<u64> = all_ids.into_iter().take(count).collect();
            let points = col.get(&sample_ids);

            let mut rows = Vec::new();
            for point in points.into_iter().flatten().take(count) {
                let mut row = helpers::point_payload_to_row(point.id, &point.payload);
                row.insert(
                    "vector".to_string(),
                    serde_json::json!(vector_preview(&point.vector)),
                );
                rows.push(row);
            }

            print_sample_rows(&rows, name, "");
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let edges = col.get_edges(None);
            let unique_ids = graph_display::unique_node_ids(&edges);
            let sample_ids: Vec<u64> = unique_ids.into_iter().take(count).collect();

            let mut rows = Vec::new();
            for node_id in &sample_ids {
                let payload = col.get_node_payload(*node_id).ok().flatten();
                rows.push(helpers::point_payload_to_row(*node_id, &payload));
            }

            print_sample_rows(&rows, name, " (Graph)");
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            let all_ids = col.all_ids();
            let sample_ids: Vec<u64> = all_ids.into_iter().take(count).collect();
            let points = col.get(&sample_ids);

            let rows: Vec<_> = points
                .into_iter()
                .flatten()
                .take(count)
                .map(|p| helpers::point_payload_to_row(p.id, &p.payload))
                .collect();

            print_sample_rows(&rows, name, " (Metadata)");
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_browse(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .browse <collection_name> [page]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    // Clamp page to >= 1 to prevent arithmetic underflow on (page - 1)
    let page: usize = parts
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);
    let page_size = 10;
    let offset = (page - 1) * page_size;

    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            browse_id_based(
                col.all_ids(),
                |ids| col.get(ids),
                name,
                "",
                page,
                page_size,
                offset,
            );
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let node_page = match graph_display::paginate_graph_nodes(&col, page, page_size) {
                Ok(p) => p,
                Err(e) => return CommandResult::Error(format!("{e}")),
            };

            println!(
                "\n{} (Graph) - Page {}/{} ({} unique nodes)",
                name.green(),
                node_page.page,
                node_page.total_pages.max(1),
                node_page.total_nodes,
            );
            println!();

            if node_page.entries.is_empty() {
                println!("No nodes on this page.\n");
            } else {
                let rows = node_entries_to_rows(&node_page.entries);
                crate::repl_output::print_table(&rows);
                println!(
                    "\nUse {} to see next page\n",
                    format!(".browse {} {}", name, page + 1).yellow()
                );
            }
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            browse_id_based(
                col.all_ids(),
                |ids| col.get(ids),
                name,
                " (Metadata)",
                page,
                page_size,
                offset,
            );
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_nodes(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .nodes <collection_name> [page]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let page: usize = parts
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);
    let page_size = 10;

    let col = match db.get_graph_collection(name) {
        Some(c) => c,
        None => return CommandResult::Error(format!("Graph collection '{name}' not found")),
    };

    let node_page = match graph_display::paginate_graph_nodes(&col, page, page_size) {
        Ok(p) => p,
        Err(e) => return CommandResult::Error(format!("{e}")),
    };

    println!(
        "\n{} in '{}' — Page {}/{} ({} unique nodes from {} edges)",
        "Nodes".bold().underline(),
        name.green(),
        node_page.page,
        node_page.total_pages.max(1),
        node_page.total_nodes,
        node_page.total_edges,
    );
    println!();

    if node_page.entries.is_empty() {
        println!("No nodes on this page.\n");
    } else {
        let rows = node_entries_to_rows(&node_page.entries);
        crate::repl_output::print_table(&rows);
        println!(
            "\nUse {} to see next page\n",
            format!(".nodes {} {}", name, page + 1).yellow()
        );
    }
    CommandResult::Continue
}

pub(crate) fn cmd_stats(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .stats <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let cfg = col.config();
            println!("\n{}", "Collection Statistics".bold().underline());
            println!("  {} {}", "Name:".cyan(), cfg.name.green());
            println!("  {} {}", "Type:".cyan(), "Vector".green());
            println!("  {} {}", "Point Count:".cyan(), cfg.point_count);
            println!("  {} {}", "Dimension:".cyan(), cfg.dimension);
            println!("  {} {:?}", "Metric:".cyan(), cfg.metric);
            println!("  {} {:?}", "Storage Mode:".cyan(), cfg.storage_mode);

            // Memory estimation
            let vector_bytes = cfg.point_count * cfg.dimension * 4;
            let id_bytes = cfg.point_count * 8;
            let total_mb = (vector_bytes + id_bytes) as f64 / 1_000_000.0;
            println!("  {} {:.2} MB", "Est. Memory:".cyan(), total_mb);
            println!();
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let edge_count = col.get_edges(None).len();
            println!("\n{}", "Collection Statistics".bold().underline());
            println!("  {} {}", "Name:".cyan(), col.name().green());
            println!("  {} {}", "Type:".cyan(), "Graph".green());
            println!("  {} {}", "Edge Count:".cyan(), edge_count);
            println!("  {} {}", "Embeddings:".cyan(), col.has_embeddings());
            println!();
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            println!("\n{}", "Collection Statistics".bold().underline());
            println!("  {} {}", "Name:".cyan(), col.name().green());
            println!("  {} {}", "Type:".cyan(), "Metadata".green());
            println!("  {} {}", "Item Count:".cyan(), col.len());
            println!();
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

// ============================================================================
// Display helpers
// ============================================================================

/// Convert graph node entries (from [`graph_display::paginate_graph_nodes`]) into
/// row maps suitable for [`crate::repl_output::print_table`].
pub(crate) fn node_entries_to_rows(
    entries: &[(u64, Option<serde_json::Value>)],
) -> Vec<HashMap<String, serde_json::Value>> {
    entries
        .iter()
        .map(|(node_id, payload)| helpers::point_payload_to_row(*node_id, payload))
        .collect()
}

/// Formats a vector as a truncated preview string (first 5 dimensions).
pub(crate) fn vector_preview(vector: &[f32]) -> String {
    let preview: Vec<f32> = vector.iter().take(5).copied().collect();
    if vector.len() > 5 {
        format!("{preview:?}... ({} dims)", vector.len())
    } else {
        format!("{preview:?}")
    }
}

/// Prints sample rows with a type suffix (empty for Vector, " (Graph)", etc.).
fn print_sample_rows(rows: &[HashMap<String, serde_json::Value>], name: &str, type_suffix: &str) {
    if rows.is_empty() {
        println!("No records found.\n");
    } else {
        println!(
            "\n{} sample(s) from {}{}:\n",
            rows.len(),
            name.green(),
            type_suffix
        );
        crate::repl_output::print_table(rows);
        println!();
    }
}

/// Prints a browse page with a consistent header and navigation hint.
/// Paginate and display an ID-based collection (Vector or Metadata).
fn browse_id_based(
    all_ids: Vec<u64>,
    get_fn: impl Fn(&[u64]) -> Vec<Option<velesdb_core::Point>>,
    name: &str,
    suffix: &str,
    page: usize,
    page_size: usize,
    offset: usize,
) {
    let total = all_ids.len();
    let total_pages = total.div_ceil(page_size);
    let page_ids: Vec<u64> = all_ids.into_iter().skip(offset).take(page_size).collect();
    let points = get_fn(&page_ids);
    let rows: Vec<_> = points
        .into_iter()
        .flatten()
        .take(page_size)
        .map(|p| helpers::point_payload_to_browse_row(p.id, &p.payload))
        .collect();
    print_browse_page(name, suffix, page, total_pages, total, &rows);
}

fn print_browse_page(
    name: &str,
    type_suffix: &str,
    page: usize,
    total_pages: usize,
    total: usize,
    rows: &[HashMap<String, serde_json::Value>],
) {
    println!(
        "\n{}{} - Page {}/{} ({} total records)",
        name.green(),
        type_suffix,
        page,
        total_pages.max(1),
        total
    );
    println!();

    if rows.is_empty() {
        println!("No records on this page.\n");
    } else {
        crate::repl_output::print_table(rows);
        println!(
            "\nUse {} to see next page\n",
            format!(".browse {} {}", name, page + 1).yellow()
        );
    }
}
