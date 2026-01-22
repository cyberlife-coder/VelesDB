//! REPL command handlers extracted from repl.rs
//!
//! Each command is implemented as a separate function for maintainability.

#![allow(clippy::doc_markdown)]

use colored::Colorize;
use instant::Instant;
use velesdb_core::Database;

use crate::repl::{OutputFormat, ReplConfig};

/// Result of a REPL command execution.
pub enum CommandResult {
    Continue,
    Quit,
    Error(String),
}

/// Handle a REPL command (line starting with '.')
#[allow(clippy::too_many_lines)]
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
        // Session commands (backslash style)
        "\\set" | ".set" => cmd_set(config, &parts),
        "\\show" | ".show" => cmd_show(config, &parts),
        "\\reset" | ".reset" => cmd_reset(config, &parts),
        "\\use" | ".use" => cmd_use(db, config, &parts),
        "\\info" | ".info" => cmd_info(db, config),
        _ => CommandResult::Error(format!("Unknown command: {cmd}")),
    }
}

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
    match db.get_collection(name) {
        Some(col) => {
            let cfg = col.config();
            println!("{} {}", "Collection:".bold(), cfg.name.green());
            println!("  Dimension: {}", cfg.dimension);
            println!("  Metric:    {:?}", cfg.metric);
            println!("  Points:    {}", cfg.point_count);
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

fn cmd_describe(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .describe <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    match db.get_collection(name) {
        Some(col) => {
            let cfg = col.config();
            println!("\n{}", "Collection Details".bold().underline());
            println!("  {} {}", "Name:".cyan(), cfg.name.green());
            println!("  {} {}", "Dimension:".cyan(), cfg.dimension);
            println!("  {} {:?}", "Metric:".cyan(), cfg.metric);
            println!("  {} {}", "Point Count:".cyan(), cfg.point_count);
            println!("  {} {:?}", "Storage Mode:".cyan(), cfg.storage_mode);

            // Estimate memory usage
            let vector_size = cfg.dimension * 4; // f32 = 4 bytes
            let estimated_mb = (cfg.point_count * vector_size) as f64 / 1_000_000.0;
            println!(
                "  {} {:.2} MB (vectors only)",
                "Est. Memory:".cyan(),
                estimated_mb
            );
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
    match db.get_collection(name) {
        Some(col) => {
            let count = col.config().point_count;
            println!("Count: {} records\n", count.to_string().green());
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

    match db.get_collection(name) {
        Some(col) => {
            let ids: Vec<u64> = (1..=(count as u64)).collect();
            let points = col.get(&ids);

            println!("\n{} ({} sample records):\n", name.green(), count);

            for point in points.into_iter().flatten() {
                println!("  {} {}", "ID:".cyan(), point.id);
                if let Some(serde_json::Value::Object(map)) = &point.payload {
                    for (k, v) in map {
                        let v_str = match v {
                            serde_json::Value::String(s) => {
                                if s.len() > 50 {
                                    format!("{}...", &s[..50])
                                } else {
                                    s.clone()
                                }
                            }
                            other => other.to_string(),
                        };
                        println!("    {}: {}", k.cyan(), v_str);
                    }
                }
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
    // BUG FIX: Ensure page >= 1 to prevent arithmetic underflow on (page - 1)
    let page: usize = parts
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);
    let page_size = 10;

    match db.get_collection(name) {
        Some(col) => {
            let total = col.config().point_count;
            let total_pages = total.div_ceil(page_size);
            let start = (page - 1) * page_size;

            if start >= total {
                println!("Page {} is empty (total: {} pages)\n", page, total_pages);
                return CommandResult::Continue;
            }

            let ids: Vec<u64> = ((start as u64 + 1)..=((start + page_size) as u64)).collect();
            let points = col.get(&ids);

            println!(
                "\n{} - Page {}/{} ({} total records)\n",
                name.green(),
                page,
                total_pages,
                total
            );

            for point in points.into_iter().flatten() {
                println!("  {} {}", "ID:".cyan(), point.id);
                if let Some(serde_json::Value::Object(map)) = &point.payload {
                    for (k, v) in map.iter().take(3) {
                        let v_str = match v {
                            serde_json::Value::String(s) => {
                                if s.len() > 40 {
                                    format!("{}...", &s[..40])
                                } else {
                                    s.clone()
                                }
                            }
                            other => other.to_string(),
                        };
                        println!("    {}: {}", k.cyan(), v_str);
                    }
                    if map.len() > 3 {
                        println!("    ... +{} more fields", map.len() - 3);
                    }
                }
                println!();
            }

            if page < total_pages {
                println!("Use .browse {} {} for next page\n", name, page + 1);
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

fn cmd_stats(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .stats <collection_name>\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    match db.get_collection(name) {
        Some(col) => {
            let cfg = col.config();
            println!("\n{}", "Collection Statistics".bold().underline());
            println!("  {} {}", "Name:".cyan(), cfg.name.green());
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
    let n_queries: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let k: usize = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);

    match db.get_collection(name) {
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

    match db.get_collection(name) {
        Some(col) => {
            let total = col.config().point_count;
            println!("Exporting {} records from {}...", total, name.green());

            let mut records = Vec::new();
            let batch_size = 1000;

            for batch_start in (0..total).step_by(batch_size) {
                let ids: Vec<u64> =
                    ((batch_start as u64 + 1)..=((batch_start + batch_size) as u64)).collect();
                let points = col.get(&ids);

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

            match std::fs::write(&filename, serde_json::to_string_pretty(&records).unwrap()) {
                Ok(()) => {
                    println!(
                        "{} Exported {} records to {}\n",
                        "✓".green(),
                        records.len(),
                        filename.green()
                    );
                }
                Err(e) => {
                    return CommandResult::Error(format!("Failed to write file: {e}"));
                }
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
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
        if db.get_collection(name).is_some() {
            config.session.use_collection(Some(name.to_string()));
            println!("Using collection: {}\n", name.green());
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
        .filter_map(|name| db.get_collection(name))
        .map(|col| col.config().point_count)
        .sum();
    println!("  {} {}", "Total Points:".cyan(), total_points);

    if let Some(col_name) = config.session.active_collection() {
        println!("  {} {}", "Active Collection:".cyan(), col_name.green());
    }
    println!();
    CommandResult::Continue
}

/// Print help text for REPL commands
pub fn print_help() {
    println!("\n{}", "VelesQL REPL Commands".bold().underline());
    println!();
    println!("  {}           Show this help", ".help".yellow());
    println!("  {}           Exit the REPL", ".quit".yellow());
    println!("  {}     List all collections", ".collections".yellow());
    println!("  {}     Show collection schema", ".schema <name>".yellow());
    println!(
        "  {}   Detailed collection stats",
        ".describe <name>".yellow()
    );
    println!(
        "  {}      Count records in collection",
        ".count <name>".yellow()
    );
    println!("  {}  Show N sample records", ".sample <name> [n]".yellow());
    println!(
        "  {} Browse with pagination",
        ".browse <name> [page]".yellow()
    );
    println!("  {} Export to JSON file", ".export <name> [file]".yellow());
    println!(
        "  {}       Toggle timing display",
        ".timing on|off".yellow()
    );
    println!(
        "  {}        Set output format",
        ".format table|json".yellow()
    );
    println!("  {}          Clear screen", ".clear".yellow());
    println!();
    println!("{}", "Session Commands:".bold().underline());
    println!();
    println!(
        "  {}   Set session parameter",
        "\\set <key> <value>".yellow()
    );
    println!("  {}       Show session settings", "\\show [key]".yellow());
    println!("  {}      Reset settings", "\\reset [key]".yellow());
    println!(
        "  {}     Select active collection",
        "\\use <collection>".yellow()
    );
    println!("  {}             Database information", "\\info".yellow());
    println!("  {} Quick benchmark", "\\bench <col> [n] [k]".yellow());
    println!();
    println!("{}", "Session Settings:".bold().underline());
    println!();
    println!(
        "  {} fast, balanced, accurate, high_recall, perfect",
        "mode".cyan()
    );
    println!("  {} 16-4096 (or auto from mode)", "ef_search".cyan());
    println!("  {} Query timeout in ms", "timeout_ms".cyan());
    println!("  {} Enable reranking (true/false)", "rerank".cyan());
    println!("  {} Max results per query", "max_results".cyan());
    println!();
    println!("{}", "VelesQL Examples:".bold().underline());
    println!();
    println!("  {}", "SELECT * FROM documents LIMIT 10;".italic().white());
    println!(
        "  {}",
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (mode = 'fast');"
            .italic()
            .white()
    );
    println!(
        "  {}",
        "SELECT * FROM items WHERE category = 'tech' LIMIT 20;"
            .italic()
            .white()
    );
    println!();
}
