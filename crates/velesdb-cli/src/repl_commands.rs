//! REPL command handlers extracted from repl.rs
//!
//! Each command is implemented as a separate function for maintainability.
//! `repl.rs` owns the I/O loop; this module owns the command dispatch logic.

#![allow(clippy::doc_markdown)]

use colored::Colorize;
use instant::Instant;
use std::collections::HashMap;
use velesdb_core::collection::graph::TraversalConfig;
use velesdb_core::Database;

use crate::collection_helpers;
use crate::graph_display;
use crate::repl::{OutputFormat, ReplConfig};

/// Result of a REPL command execution.
pub enum CommandResult {
    Continue,
    Quit,
    Error(String),
}

/// Handle a REPL command (line starting with '.')
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub fn handle_command(db: &Database, line: &str, config: &mut ReplConfig) -> CommandResult {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let cmd = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();

    match cmd.as_str() {
        ".quit" | ".exit" | ".q" => CommandResult::Quit,
        ".help" | ".h" => {
            print_help();
            CommandResult::Continue
        }
        ".collections" | ".tables" => cmd_collections(db),
        ".schema" => cmd_schema(db, &parts),
        ".timing" => cmd_timing(config, &parts),
        ".format" => cmd_format(config, &parts),
        ".clear" => cmd_clear(),
        ".describe" | ".desc" => cmd_describe(db, &parts),
        ".count" => cmd_count(db, &parts),
        ".sample" => cmd_sample(db, &parts),
        ".browse" => cmd_browse(db, &parts),
        ".stats" => cmd_stats(db, &parts),
        ".bench" | "\\bench" => cmd_bench(db, config, &parts),
        ".export" => cmd_export(db, &parts),
        ".nodes" => cmd_nodes(db, &parts),
        ".graph" => cmd_graph(db, &parts),
        // Phase 5 -- REPL Enhancements
        ".explain" => cmd_explain(db, &parts),
        ".analyze" => cmd_analyze(db, &parts),
        ".indexes" => cmd_indexes(db, &parts),
        ".delete" => cmd_delete(db, &parts),
        ".flush" => cmd_flush(db, &parts),
        ".create-index" => cmd_create_index(db, &parts),
        ".drop-index" => cmd_drop_index(db, &parts),
        // Phase 6 -- Advanced Features
        ".sparse-search" => cmd_sparse_search(db, &parts),
        ".hybrid-sparse" => cmd_hybrid_sparse(db, &parts),
        ".agent" => cmd_agent(&parts),
        ".guardrails" => cmd_guardrails(),
        // Session commands (backslash style)
        "\\set" | ".set" => cmd_set(config, &parts),
        "\\show" | ".show" => cmd_show(config, &parts),
        "\\reset" | ".reset" => cmd_reset(config, &parts),
        "\\use" | ".use" => cmd_use(db, config, &parts),
        "\\info" | ".info" => cmd_info(db, config),
        _ => CommandResult::Error(format!("Unknown command: {cmd}")),
    }
}

// ============================================================================
// Basic commands
// ============================================================================

fn cmd_collections(db: &Database) -> CommandResult {
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

fn cmd_schema(db: &Database, parts: &[&str]) -> CommandResult {
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

fn cmd_timing(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Timing is {}", if config.timing { "ON" } else { "OFF" });
    } else {
        match parts[1].to_lowercase().as_str() {
            "on" | "true" | "1" => {
                config.timing = true;
                println!("Timing ON");
            }
            "off" | "false" | "0" => {
                config.timing = false;
                println!("Timing OFF");
            }
            _ => {
                return CommandResult::Error("Use: .timing on|off".to_string());
            }
        }
    }
    println!();
    CommandResult::Continue
}

fn cmd_format(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Format is {:?}", config.format);
    } else {
        match parts[1].to_lowercase().as_str() {
            "table" => {
                config.format = OutputFormat::Table;
                println!("Format: table");
            }
            "json" => {
                config.format = OutputFormat::Json;
                println!("Format: json");
            }
            _ => {
                return CommandResult::Error("Use: .format table|json".to_string());
            }
        }
    }
    println!();
    CommandResult::Continue
}

fn cmd_clear() -> CommandResult {
    print!("\x1B[2J\x1B[1;1H");
    CommandResult::Continue
}

// ============================================================================
// Collection inspection commands
// ============================================================================

fn cmd_describe(db: &Database, parts: &[&str]) -> CommandResult {
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

fn cmd_count(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .count <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let count = col.config().point_count;
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

fn cmd_sample(db: &Database, parts: &[&str]) -> CommandResult {
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
                let mut row = HashMap::new();
                row.insert("id".to_string(), serde_json::json!(point.id));

                // Show vector preview (first 5 dims)
                let vec_preview: Vec<f32> = point.vector.iter().take(5).copied().collect();
                let vec_str = if point.vector.len() > 5 {
                    format!("{:?}... ({} dims)", vec_preview, point.vector.len())
                } else {
                    format!("{:?}", vec_preview)
                };
                row.insert("vector".to_string(), serde_json::json!(vec_str));

                if let Some(serde_json::Value::Object(map)) = &point.payload {
                    for (k, v) in map {
                        row.insert(k.clone(), v.clone());
                    }
                }
                rows.push(row);
            }

            if rows.is_empty() {
                println!("No records found.\n");
            } else {
                println!("\n{} sample(s) from {}:\n", rows.len(), name.green());
                crate::repl_output::print_table(&rows);
                println!();
            }
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let edges = col.get_edges(None);
            let mut unique_ids: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
            for e in &edges {
                unique_ids.insert(e.source());
                unique_ids.insert(e.target());
            }
            let sample_ids: Vec<u64> = unique_ids.into_iter().take(count).collect();

            let mut rows = Vec::new();
            for node_id in &sample_ids {
                let mut row = HashMap::new();
                row.insert("id".to_string(), serde_json::json!(node_id));
                if let Ok(Some(serde_json::Value::Object(map))) = col.get_node_payload(*node_id) {
                    for (k, v) in map {
                        row.insert(k, v);
                    }
                }
                rows.push(row);
            }

            if rows.is_empty() {
                println!("No nodes found.\n");
            } else {
                println!(
                    "\n{} sample(s) from {} (Graph):\n",
                    rows.len(),
                    name.green()
                );
                crate::repl_output::print_table(&rows);
                println!();
            }
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            let all_ids = col.all_ids();
            let sample_ids: Vec<u64> = all_ids.into_iter().take(count).collect();
            let points = col.get(&sample_ids);

            let mut rows = Vec::new();
            for point in points.into_iter().flatten().take(count) {
                let mut row = HashMap::new();
                row.insert("id".to_string(), serde_json::json!(point.id));
                if let Some(serde_json::Value::Object(map)) = &point.payload {
                    for (k, v) in map {
                        row.insert(k.clone(), v.clone());
                    }
                }
                rows.push(row);
            }

            if rows.is_empty() {
                println!("No records found.\n");
            } else {
                println!(
                    "\n{} sample(s) from {} (Metadata):\n",
                    rows.len(),
                    name.green()
                );
                crate::repl_output::print_table(&rows);
                println!();
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_browse(db: &Database, parts: &[&str]) -> CommandResult {
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
            let all_ids = col.all_ids();
            let total = all_ids.len();
            let total_pages = total.div_ceil(page_size);

            let page_ids: Vec<u64> = all_ids.into_iter().skip(offset).take(page_size).collect();
            let points = col.get(&page_ids);

            let mut rows = Vec::new();
            for point in points.into_iter().flatten().take(page_size) {
                let mut row = HashMap::new();
                row.insert("id".to_string(), serde_json::json!(point.id));

                if let Some(serde_json::Value::Object(map)) = &point.payload {
                    for (k, v) in map {
                        // Truncate long values at 47 chars
                        let display_val = match v {
                            serde_json::Value::String(s) if s.len() > 50 => {
                                let truncated: String = s.chars().take(47).collect();
                                serde_json::json!(format!("{truncated}..."))
                            }
                            other => other.clone(),
                        };
                        row.insert(k.clone(), display_val);
                    }
                }
                rows.push(row);
            }

            println!(
                "\n{} - Page {}/{} ({} total records)",
                name.green(),
                page,
                total_pages.max(1),
                total
            );
            println!();

            if rows.is_empty() {
                println!("No records on this page.\n");
            } else {
                crate::repl_output::print_table(&rows);
                println!(
                    "\nUse {} to see next page\n",
                    format!(".browse {} {}", name, page + 1).yellow()
                );
            }
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
            let all_ids = col.all_ids();
            let total = all_ids.len();
            let total_pages = total.div_ceil(page_size);

            let page_ids: Vec<u64> = all_ids.into_iter().skip(offset).take(page_size).collect();
            let points = col.get(&page_ids);

            let mut rows = Vec::new();
            for point in points.into_iter().flatten().take(page_size) {
                let mut row = HashMap::new();
                row.insert("id".to_string(), serde_json::json!(point.id));

                if let Some(serde_json::Value::Object(map)) = &point.payload {
                    for (k, v) in map {
                        let display_val = match v {
                            serde_json::Value::String(s) if s.len() > 50 => {
                                let truncated: String = s.chars().take(47).collect();
                                serde_json::json!(format!("{truncated}..."))
                            }
                            other => other.clone(),
                        };
                        row.insert(k.clone(), display_val);
                    }
                }
                rows.push(row);
            }

            println!(
                "\n{} (Metadata) - Page {}/{} ({} total records)",
                name.green(),
                page,
                total_pages.max(1),
                total
            );
            println!();

            if rows.is_empty() {
                println!("No records on this page.\n");
            } else {
                crate::repl_output::print_table(&rows);
                println!(
                    "\nUse {} to see next page\n",
                    format!(".browse {} {}", name, page + 1).yellow()
                );
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_nodes(db: &Database, parts: &[&str]) -> CommandResult {
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

fn cmd_stats(db: &Database, parts: &[&str]) -> CommandResult {
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

fn cmd_bench(db: &Database, config: &ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .bench <collection_name> [n_queries] [k]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let n_queries: usize = parts
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100)
        .max(1);
    let k: usize = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);

    match db.get_vector_collection(name) {
        Some(col) => {
            let cfg = col.config();
            println!(
                "\nBenchmarking {} ({} points, {}D)...\n",
                name.green(),
                cfg.point_count,
                cfg.dimension
            );
            println!(
                "  {} queries, k={}, mode={:?}",
                n_queries,
                k,
                config.session.mode()
            );

            // Generate random query vectors
            let start = Instant::now();
            let mut total_results = 0usize;

            for i in 0..n_queries {
                // Use deterministic pseudo-random for reproducibility
                let query: Vec<f32> = (0..cfg.dimension)
                    .map(|j| ((i * 31 + j * 17) % 1000) as f32 / 1000.0)
                    .collect();

                if let Ok(results) = col.search(&query, k) {
                    total_results += results.len();
                }
            }

            let elapsed = start.elapsed();
            let qps = n_queries as f64 / elapsed.as_secs_f64();
            let avg_latency_ms = elapsed.as_millis() as f64 / n_queries as f64;

            println!("\n{}", "Results:".bold());
            println!("  {} {:.2} queries/sec", "Throughput:".cyan(), qps);
            println!("  {} {:.2} ms", "Avg Latency:".cyan(), avg_latency_ms);
            println!("  {} {} results", "Total Results:".cyan(), total_results);
            println!();
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_export(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .export <collection_name> [filename.json]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let filename = parts
        .get(2)
        .map_or_else(|| format!("{name}.json"), std::string::ToString::to_string);

    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let all_ids = col.all_ids();
            let total = all_ids.len();
            println!("Exporting {} records from {}...", total, name.green());

            let mut records = Vec::new();
            let batch_size = 1000;

            for batch in all_ids.chunks(batch_size) {
                let points = col.get(batch);

                for point in points.into_iter().flatten() {
                    let mut record = serde_json::Map::new();
                    record.insert("id".to_string(), serde_json::json!(point.id));
                    record.insert("vector".to_string(), serde_json::json!(point.vector));
                    if let Some(payload) = &point.payload {
                        record.insert("payload".to_string(), payload.clone());
                    }
                    records.push(serde_json::Value::Object(record));
                }
            }

            let json_str = match serde_json::to_string_pretty(&records) {
                Ok(s) => s,
                Err(e) => {
                    return CommandResult::Error(format!("Failed to serialize records: {e}"));
                }
            };
            match std::fs::write(&filename, json_str) {
                Ok(()) => {
                    println!(
                        "{} Exported {} records to {}\n",
                        "\u{2713}".green(),
                        records.len(),
                        filename.green()
                    );
                }
                Err(e) => {
                    return CommandResult::Error(format!("Failed to write file: {e}"));
                }
            }
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            let all_ids = col.all_ids();
            let total = all_ids.len();
            println!(
                "Exporting {} records from {} (Metadata)...",
                total,
                name.green()
            );

            let mut records = Vec::new();
            let batch_size = 1000;

            for batch in all_ids.chunks(batch_size) {
                let points = col.get(batch);

                for point in points.into_iter().flatten() {
                    let mut record = serde_json::Map::new();
                    record.insert("id".to_string(), serde_json::json!(point.id));
                    if let Some(payload) = &point.payload {
                        record.insert("payload".to_string(), payload.clone());
                    }
                    records.push(serde_json::Value::Object(record));
                }
            }

            let json_str = match serde_json::to_string_pretty(&records) {
                Ok(s) => s,
                Err(e) => {
                    return CommandResult::Error(format!("Failed to serialize records: {e}"));
                }
            };
            match std::fs::write(&filename, json_str) {
                Ok(()) => {
                    println!(
                        "{} Exported {} records to {}\n",
                        "\u{2713}".green(),
                        records.len(),
                        filename.green()
                    );
                }
                Err(e) => {
                    return CommandResult::Error(format!("Failed to write file: {e}"));
                }
            }
        }
        Some(collection_helpers::TypedCollection::Graph(_)) => {
            return CommandResult::Error(
                "Export is not supported for Graph collections. Use 'velesdb graph get-edges' instead.".to_string()
            );
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

// ============================================================================
// Phase 5 -- REPL Enhancement commands
// ============================================================================

fn cmd_explain(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .explain <VelesQL query>\n");
        return CommandResult::Continue;
    }
    // Rejoin everything after ".explain" as the query string
    let query_str = parts[1..].join(" ");

    let parsed = match velesdb_core::velesql::Parser::parse(&query_str) {
        Ok(q) => q,
        Err(e) => return CommandResult::Error(format!("Parse error: {}", e.message)),
    };

    match db.explain_query(&parsed) {
        Ok(plan) => {
            println!("\n{}", plan.to_tree());
        }
        Err(e) => return CommandResult::Error(format!("Explain error: {e}")),
    }
    CommandResult::Continue
}

fn cmd_analyze(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .analyze <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];

    match db.analyze_collection(name) {
        Ok(stats) => {
            println!("\n{}", "Collection Analysis".bold().underline());
            println!("  {} {}", "Collection:".cyan(), name.green());
            println!("  {} {}", "Total Points:".cyan(), stats.total_points);
            println!("  {} {}", "Row Count:".cyan(), stats.row_count);
            println!("  {} {}", "Deleted:".cyan(), stats.deleted_count);
            println!("  {} {}", "Live Rows:".cyan(), stats.live_row_count());
            println!(
                "  {} {:.1}%",
                "Deletion Ratio:".cyan(),
                stats.deletion_ratio() * 100.0
            );
            println!(
                "  {} {} bytes",
                "Payload Size:".cyan(),
                stats.payload_size_bytes
            );
            println!(
                "  {} {} bytes",
                "Total Size:".cyan(),
                stats.total_size_bytes
            );
            println!(
                "  {} {} bytes",
                "Avg Row Size:".cyan(),
                stats.avg_row_size_bytes
            );

            if !stats.field_stats.is_empty() {
                println!("\n  {}", "Field Statistics:".bold());
                for (field, fs) in &stats.field_stats {
                    println!(
                        "    {} distinct={}, null={}",
                        field.cyan(),
                        fs.distinct_values,
                        fs.null_count
                    );
                }
            }

            if !stats.index_stats.is_empty() {
                println!("\n  {}", "Index Statistics:".bold());
                for (idx_name, is) in &stats.index_stats {
                    println!(
                        "    {} entries={}, size={} bytes",
                        idx_name.cyan(),
                        is.entry_count,
                        is.size_bytes
                    );
                }
            }
            println!();
        }
        Err(e) => return CommandResult::Error(format!("Analyze error: {e}")),
    }
    CommandResult::Continue
}

fn cmd_indexes(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .indexes <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];

    match db.get_vector_collection(name) {
        Some(col) => {
            let indexes = col.list_indexes();
            if indexes.is_empty() {
                println!("No indexes on collection '{}'.\n", name.green());
            } else {
                println!("\n{} ({})\n", "Indexes".bold().underline(), name.green());
                for idx in &indexes {
                    println!(
                        "  {} {}.{} (cardinality={}, mem={} bytes)",
                        idx.index_type.cyan(),
                        idx.label,
                        idx.property.green(),
                        idx.cardinality,
                        idx.memory_bytes,
                    );
                }
                println!("\n  Total: {} index(es)\n", indexes.len());
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_delete(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 3 {
        println!("Usage: .delete <collection_name> <id> [id2 ...]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];

    let mut ids = Vec::new();
    for id_str in &parts[2..] {
        match id_str.parse::<u64>() {
            Ok(id) => ids.push(id),
            Err(_) => return CommandResult::Error(format!("Invalid ID: {id_str}")),
        }
    }

    match db.get_vector_collection(name) {
        Some(col) => match col.delete(&ids) {
            Ok(()) => {
                println!(
                    "{} Deleted {} point(s) from {}\n",
                    "\u{2713}".green(),
                    ids.len(),
                    name.green()
                );
            }
            Err(e) => return CommandResult::Error(format!("Delete error: {e}")),
        },
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_flush(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .flush <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];

    match db.get_vector_collection(name) {
        Some(col) => match col.flush() {
            Ok(()) => {
                println!(
                    "{} Collection '{}' flushed to disk.\n",
                    "\u{2713}".green(),
                    name.green()
                );
            }
            Err(e) => return CommandResult::Error(format!("Flush error: {e}")),
        },
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_create_index(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 3 {
        println!("Usage: .create-index <collection> <field> [--type secondary|property|range]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let field = parts[2];
    let idx_type = parse_flag(parts, "--type").unwrap_or_else(|| "secondary".to_string());

    match db.get_vector_collection(name) {
        Some(col) => {
            let result = match idx_type.as_str() {
                "property" | "hash" => col.create_property_index(field, field),
                "range" => col.create_range_index(field, field),
                _ => col.create_index(field),
            };
            match result {
                Ok(()) => {
                    println!(
                        "{} Created {} index on '{}' in {}\n",
                        "\u{2713}".green(),
                        idx_type.cyan(),
                        field.green(),
                        name.green(),
                    );
                }
                Err(e) => return CommandResult::Error(format!("Create index error: {e}")),
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_drop_index(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .drop-index <collection> <label> <property>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let label = parts[2];
    let property = parts[3];

    match db.get_vector_collection(name) {
        Some(col) => match col.drop_index(label, property) {
            Ok(true) => {
                println!(
                    "{} Dropped index {}.{} from {}\n",
                    "\u{2713}".green(),
                    label,
                    property.green(),
                    name.green(),
                );
            }
            Ok(false) => {
                println!(
                    "No index found for {}.{} on {}.\n",
                    label,
                    property,
                    name.green()
                );
            }
            Err(e) => return CommandResult::Error(format!("Drop index error: {e}")),
        },
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

// ============================================================================
// Phase 6 -- Advanced Feature commands
// ============================================================================

fn cmd_sparse_search(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .sparse-search <collection> <index_name> <json_sparse_vector> [k]\n");
        println!(
            "  JSON sparse vector format: {}",
            "[[index, weight], ...]".italic().white()
        );
        println!(
            "  Example: {} my_col \"\" [[0,1.5],[3,0.8]] 10\n",
            ".sparse-search".yellow()
        );
        return CommandResult::Continue;
    }
    let name = parts[1];
    let index_name = parts[2];
    let sparse_json = parts[3];
    let k: usize = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);

    let pairs: Vec<(u32, f32)> = match serde_json::from_str(sparse_json) {
        Ok(p) => p,
        Err(e) => {
            return CommandResult::Error(format!(
                "Invalid sparse vector JSON: {e}\nExpected format: [[index, weight], ...]"
            ));
        }
    };
    let sparse_vec = velesdb_core::sparse_index::SparseVector::new(pairs);

    match db.get_vector_collection(name) {
        Some(col) => match col.sparse_search(&sparse_vec, k, index_name) {
            Ok(results) => {
                if results.is_empty() {
                    println!("No results found.\n");
                } else {
                    println!(
                        "\n{} ({} results)\n",
                        "Sparse Search Results".bold().underline(),
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
            Err(e) => return CommandResult::Error(format!("Sparse search error: {e}")),
        },
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

#[allow(clippy::too_many_lines)]
fn cmd_hybrid_sparse(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!(
            "Usage: .hybrid-sparse <collection> <dense_json> <sparse_json> [k] \
             [--strategy rrf|average|max] [--index <name>]\n"
        );
        println!("  Dense vector:  {}", "[0.1, 0.2, ...]".italic().white());
        println!(
            "  Sparse vector: {}",
            "[[index, weight], ...]".italic().white()
        );
        println!(
            "  Example: {} docs [0.1,0.2,0.3,0.4] [[0,1.5],[3,0.8]] 10 --strategy rrf\n",
            ".hybrid-sparse".yellow()
        );
        return CommandResult::Continue;
    }
    let name = parts[1];
    let dense_json = parts[2];
    let sparse_json = parts[3];
    let k: usize = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);

    let strategy_str = parse_flag(parts, "--strategy").unwrap_or_else(|| "rrf".to_string());
    let index_name = parse_flag(parts, "--index").unwrap_or_default();

    let dense_vector: Vec<f32> = match serde_json::from_str(dense_json) {
        Ok(v) => v,
        Err(e) => {
            return CommandResult::Error(format!(
                "Invalid dense vector JSON: {e}\nExpected format: [0.1, 0.2, ...]"
            ));
        }
    };

    let sparse_pairs: Vec<(u32, f32)> = match serde_json::from_str(sparse_json) {
        Ok(p) => p,
        Err(e) => {
            return CommandResult::Error(format!(
                "Invalid sparse vector JSON: {e}\nExpected format: [[index, weight], ...]"
            ));
        }
    };
    let sparse_vec = velesdb_core::sparse_index::SparseVector::new(sparse_pairs);

    let strategy = match strategy_str.as_str() {
        "rrf" => velesdb_core::FusionStrategy::rrf_default(),
        "average" => velesdb_core::FusionStrategy::Average,
        "max" | "maximum" => velesdb_core::FusionStrategy::Maximum,
        other => {
            return CommandResult::Error(format!(
                "Unknown fusion strategy: '{other}'. Use rrf, average, or max."
            ));
        }
    };

    match db.get_vector_collection(name) {
        Some(col) => {
            match col.hybrid_sparse_search(&dense_vector, &sparse_vec, k, &index_name, &strategy) {
                Ok(results) => {
                    if results.is_empty() {
                        println!("No results found.\n");
                    } else {
                        println!(
                            "\n{} ({} results, strategy={})\n",
                            "Hybrid Sparse Search Results".bold().underline(),
                            results.len(),
                            strategy_str.cyan()
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
                Err(e) => return CommandResult::Error(format!("Hybrid search error: {e}")),
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_agent(parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        print_agent_help();
        return CommandResult::Continue;
    }
    match parts[1] {
        "help" => print_agent_help(),
        sub => {
            println!(
                "{} Agent subcommand '{}' is not yet implemented.\n\
                 Agent memory management is primarily used via SDK/server.\n",
                "\u{2139}".cyan(),
                sub
            );
        }
    }
    CommandResult::Continue
}

fn print_agent_help() {
    println!("\n{}", "Agent Commands (Preview)".bold().underline());
    println!();
    println!("  Agent memory management is primarily used via the SDK or REST server.");
    println!("  CLI support is planned for a future release.\n");
    println!("  Planned subcommands:");
    println!("    {} Store a memory entry", ".agent store".yellow());
    println!("    {} Recall memories", ".agent recall".yellow());
    println!("    {} List stored memories", ".agent list".yellow());
    println!("    {} Clear agent memory", ".agent clear".yellow());
    println!();
}

fn cmd_guardrails() -> CommandResult {
    let limits = velesdb_core::guardrails::QueryLimits::default();
    println!("\n{}", "Query Guard-Rails Configuration".bold().underline());
    println!();
    println!("  {} {} ms", "Timeout:".cyan(), limits.timeout_ms);
    println!("  {} {}", "Max Depth:".cyan(), limits.max_depth);
    println!("  {} {}", "Max Cardinality:".cyan(), limits.max_cardinality);
    println!(
        "  {} {} bytes ({:.0} MB)",
        "Memory Limit:".cyan(),
        limits.memory_limit_bytes,
        limits.memory_limit_bytes as f64 / (1024.0 * 1024.0)
    );
    println!("  {} {} qps", "Rate Limit:".cyan(), limits.rate_limit_qps);
    println!(
        "  {} {} failures",
        "Circuit Breaker Threshold:".cyan(),
        limits.circuit_failure_threshold
    );
    println!(
        "  {} {} s",
        "Circuit Recovery:".cyan(),
        limits.circuit_recovery_seconds
    );
    println!();
    CommandResult::Continue
}

// ============================================================================
// Session Commands
// ============================================================================

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn cmd_set(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 3 {
        println!("Usage: \\set <setting> <value>\n");
        println!("Settings: mode, ef_search, timeout_ms, rerank, max_results\n");
        return CommandResult::Continue;
    }
    let key = parts[1];
    let value = parts[2];
    match config.session.set(key, value) {
        Ok(()) => println!("{} = {}\n", key.cyan(), value.green()),
        Err(e) => return CommandResult::Error(e),
    }
    CommandResult::Continue
}

fn cmd_show(config: &ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("\n{}", "Session Settings".bold().underline());
        for (key, value) in config.session.all_settings() {
            println!("  {} = {}", key.cyan(), value.green());
        }
        println!();
    } else {
        let key = parts[1];
        match config.session.get(key) {
            Some(value) => println!("{} = {}\n", key.cyan(), value.green()),
            None => return CommandResult::Error(format!("Unknown setting: {key}")),
        }
    }
    CommandResult::Continue
}

fn cmd_reset(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    let key = parts.get(1).copied();
    config.session.reset(key);
    if let Some(k) = key {
        println!("Reset {}\n", k.cyan());
    } else {
        println!("All settings reset to defaults\n");
    }
    CommandResult::Continue
}

fn cmd_use(db: &Database, config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        match config.session.active_collection() {
            Some(name) => println!("Active collection: {}\n", name.green()),
            None => println!("No active collection. Usage: \\use <collection>\n"),
        }
    } else {
        let name = parts[1];
        if collection_helpers::resolve_collection(db, name).is_some() {
            config.session.use_collection(Some(name.to_string()));
            println!(
                "Using collection: {} [{}]\n",
                name.green(),
                collection_helpers::collection_type_label(db, name).cyan()
            );
        } else {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_info(db: &Database, config: &ReplConfig) -> CommandResult {
    println!("\n{}", "VelesDB Information".bold().underline());
    println!("  {} {}", "Version:".cyan(), VERSION.green());
    println!("  {} {}", "Database:".cyan(), "active".green());

    let collections = db.list_collections();
    println!("  {} {}", "Collections:".cyan(), collections.len());

    let total_points: usize = collections
        .iter()
        .filter_map(|name| db.get_vector_collection(name))
        .map(|col| col.config().point_count)
        .sum();
    println!("  {} {}", "Total Points:".cyan(), total_points);

    if let Some(col_name) = config.session.active_collection() {
        println!("  {} {}", "Active Collection:".cyan(), col_name.green());
    }
    println!();
    CommandResult::Continue
}

// ============================================================================
// Graph REPL commands
// ============================================================================

fn cmd_graph(db: &Database, parts: &[&str]) -> CommandResult {
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
    let node_id: u64 = match parts[3].parse() {
        Ok(v) => v,
        Err(_) => return CommandResult::Error(format!("Invalid node ID: {}", parts[3])),
    };

    let (in_deg, out_deg) = col.node_degree(node_id);
    graph_display::print_degree(node_id, in_deg, out_deg);
    CommandResult::Continue
}

fn cmd_graph_traverse(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .graph traverse <collection> <source> [--algo bfs|dfs] [--depth N] [--limit N]\n");
        return CommandResult::Continue;
    }
    let col = match resolve_graph_collection(db, parts, 2) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let source: u64 = match parts[3].parse() {
        Ok(v) => v,
        Err(_) => return CommandResult::Error(format!("Invalid source ID: {}", parts[3])),
    };

    let algo = parse_flag(parts, "--algo").unwrap_or_else(|| "bfs".to_string());
    let max_depth: u32 = parse_flag(parts, "--depth")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let limit: usize = parse_flag(parts, "--limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let config = TraversalConfig {
        min_depth: 1,
        max_depth,
        limit,
        rel_types: Vec::new(),
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
    let node_id: u64 = match parts[3].parse() {
        Ok(v) => v,
        Err(_) => return CommandResult::Error(format!("Invalid node ID: {}", parts[3])),
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

// ============================================================================
// Shared conversion helpers
// ============================================================================

/// Convert graph node entries (from [`graph_display::paginate_graph_nodes`]) into
/// row maps suitable for [`crate::repl_output::print_table`].
fn node_entries_to_rows(
    entries: &[(u64, Option<serde_json::Value>)],
) -> Vec<HashMap<String, serde_json::Value>> {
    entries
        .iter()
        .map(|(node_id, payload)| {
            let mut row = HashMap::new();
            row.insert("id".to_string(), serde_json::json!(node_id));
            if let Some(serde_json::Value::Object(map)) = payload {
                for (k, v) in map {
                    row.insert(k.clone(), v.clone());
                }
            }
            row
        })
        .collect()
}

// ============================================================================
// Utilities
// ============================================================================

/// Parse a `--flag value` pair from command parts.
fn parse_flag(parts: &[&str], flag: &str) -> Option<String> {
    parts
        .iter()
        .position(|&p| p == flag)
        .and_then(|i| parts.get(i + 1))
        .map(|s| s.to_string())
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
    println!();
}

/// Print help text for REPL commands (delegates to repl_output for the full text).
pub fn print_help() {
    crate::repl_output::print_help();
}
