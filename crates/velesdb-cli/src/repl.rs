#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! REPL (Read-Eval-Print-Loop) for `VelesQL` queries

use anyhow::{Context, Result};
use colored::Colorize;
// NOTE: comfy_table imports moved to repl_output.rs (EPIC-061/US-004 refactoring)
use instant::Instant;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Completer, Editor, Helper, Highlighter, Hinter, Validator};
use std::collections::HashMap;
use std::path::PathBuf;
use velesdb_core::Database;

use crate::session::SessionSettings;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// REPL configuration
#[derive(Debug, Clone)]
pub struct ReplConfig {
    pub timing: bool,
    pub format: OutputFormat,
    pub session: SessionSettings,
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            timing: true,
            format: OutputFormat::Table,
            session: SessionSettings::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Table,
    Json,
}

/// Query execution result
#[derive(Debug)]
pub struct QueryResult {
    pub rows: Vec<HashMap<String, serde_json::Value>>,
    pub duration_ms: f64,
}

#[derive(Completer, Helper, Highlighter, Hinter, Validator)]
struct ReplHelper;

/// Run the interactive REPL
#[allow(clippy::needless_pass_by_value)] // PathBuf ownership required for Database::open
pub fn run(path: PathBuf) -> Result<()> {
    println!(
        "\n{}",
        format!("VelesDB v{VERSION} - VelesQL REPL").bold().cyan()
    );
    println!("Database: {}", path.display().to_string().green());
    println!(
        "Type {} for commands, {} to exit\n",
        ".help".yellow(),
        ".quit".yellow()
    );

    let db = Database::open(&path).context("Failed to open database")?;

    let mut rl: Editor<ReplHelper, DefaultHistory> = Editor::new()?;
    rl.set_helper(Some(ReplHelper));

    let history_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".velesdb_history");
    let _ = rl.load_history(&history_path);

    let mut config = ReplConfig::default();

    loop {
        let prompt = "velesdb> ".bold().blue().to_string();
        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(line);

                if line.starts_with('.') {
                    match handle_command(&db, line, &mut config) {
                        CommandResult::Continue => (),
                        CommandResult::Quit => break,
                        CommandResult::Error(e) => {
                            println!("{} {}", "Error:".red().bold(), e);
                        }
                    }
                } else {
                    match execute_query(&db, line) {
                        Ok(result) => {
                            print_result(&result, &format!("{:?}", config.format).to_lowercase());
                            if config.timing {
                                println!(
                                    "\n{} rows ({:.2}ms)\n",
                                    result.rows.len().to_string().green(),
                                    result.duration_ms
                                );
                            }
                        }
                        Err(e) => {
                            println!("{} {}\n", "Error:".red().bold(), e);
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Use .quit to exit");
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("{} {:?}", "Error:".red().bold(), err);
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    println!("Goodbye!");
    Ok(())
}

enum CommandResult {
    Continue,
    Quit,
    Error(String),
}

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Reason: REPL command dispatcher with many branches
fn handle_command(db: &Database, line: &str, config: &mut ReplConfig) -> CommandResult {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let cmd = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();

    match cmd.as_str() {
        ".quit" | ".exit" | ".q" => CommandResult::Quit,

        ".help" | ".h" => {
            print_help();
            CommandResult::Continue
        }

        ".collections" | ".tables" => {
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

        ".schema" => {
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

        ".timing" => {
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

        ".format" => {
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

        ".clear" => {
            print!("\x1B[2J\x1B[1;1H");
            CommandResult::Continue
        }

        ".describe" | ".desc" => {
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

        ".count" => {
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

        ".sample" => {
            if parts.len() < 2 {
                println!("Usage: .sample <collection_name> [count]\n");
                return CommandResult::Continue;
            }
            let name = parts[1];
            let count: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);

            match db.get_collection(name) {
                Some(col) => {
                    let ids: Vec<u64> = (1..=(count as u64 * 2)).collect();
                    let points = col.get(&ids);

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
                None => {
                    return CommandResult::Error(format!("Collection '{name}' not found"));
                }
            }
            CommandResult::Continue
        }

        ".browse" => {
            if parts.len() < 2 {
                println!("Usage: .browse <collection_name> [page]\n");
                return CommandResult::Continue;
            }
            let name = parts[1];
            let page: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            let page_size = 10;
            let offset = (page - 1) * page_size;

            match db.get_collection(name) {
                Some(col) => {
                    let total = col.config().point_count;
                    let total_pages = total.div_ceil(page_size);

                    let ids: Vec<u64> =
                        ((offset as u64 + 1)..=(offset as u64 + page_size as u64 * 2)).collect();
                    let points = col.get(&ids);

                    let mut rows = Vec::new();
                    for point in points.into_iter().flatten().take(page_size) {
                        let mut row = HashMap::new();
                        row.insert("id".to_string(), serde_json::json!(point.id));

                        if let Some(serde_json::Value::Object(map)) = &point.payload {
                            for (k, v) in map {
                                // Truncate long values
                                let display_val = match v {
                                    serde_json::Value::String(s) if s.len() > 50 => {
                                        serde_json::json!(format!("{}...", &s[..47]))
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
                None => {
                    return CommandResult::Error(format!("Collection '{name}' not found"));
                }
            }
            CommandResult::Continue
        }

        // ========== Session commands (backslash style) ==========
        "\\set" | ".set" => {
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

        "\\show" | ".show" => {
            if parts.len() < 2 {
                // Show all settings
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

        "\\reset" | ".reset" => {
            let key = parts.get(1).copied();
            config.session.reset(key);
            if let Some(k) = key {
                println!("Reset {}\n", k.cyan());
            } else {
                println!("All settings reset to defaults\n");
            }
            CommandResult::Continue
        }

        "\\use" | ".use" => {
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

        "\\info" | ".info" => {
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

        "\\bench" | ".bench" => {
            if parts.len() < 2 {
                println!("Usage: \\bench <collection> [n_queries] [k]\n");
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

        ".export" => {
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
                        let ids: Vec<u64> = ((batch_start as u64 + 1)
                            ..=((batch_start + batch_size) as u64))
                            .collect();
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

                    match std::fs::write(&filename, serde_json::to_string_pretty(&records).unwrap())
                    {
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

        _ => CommandResult::Error(format!("Unknown command: {cmd}")),
    }
}

// NOTE: print_help moved to repl_output.rs (EPIC-061/US-004 refactoring)
fn print_help() {
    crate::repl_output::print_help();
}

/// Execute a `VelesQL` query and return results
pub fn execute_query(db: &Database, query: &str) -> Result<QueryResult> {
    let start = Instant::now();

    // Parse the query
    let parsed = velesdb_core::velesql::Parser::parse(query)
        .map_err(|e| anyhow::anyhow!("Parse error: {}", e.message))?;

    let collection_name = &parsed.select.from;

    // Get the collection
    let collection = db
        .get_collection(collection_name)
        .ok_or_else(|| anyhow::anyhow!("Collection '{collection_name}' not found"))?;

    // Check if there's a vector search requiring parameters
    let has_param_vector = parsed
        .select
        .where_clause
        .as_ref()
        .is_some_and(contains_param_vector);

    if has_param_vector {
        // Vector search with parameter requires external input
        println!(
            "{}",
            "Note: Vector search with $parameter requires REST API. Use literal vectors or metadata-only queries."
                .yellow()
        );
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        return Ok(QueryResult {
            rows: Vec::new(),
            duration_ms,
        });
    }

    // Use unified execute_query from Collection (empty params for CLI)
    let params = HashMap::new();
    let results = collection
        .execute_query(&parsed, &params)
        .map_err(|e| anyhow::anyhow!("Query error: {e}"))?;

    // Convert SearchResult to row format
    let rows: Vec<HashMap<String, serde_json::Value>> = results
        .into_iter()
        .map(|r| {
            let mut row = HashMap::new();
            row.insert("id".to_string(), serde_json::json!(r.point.id));
            row.insert("score".to_string(), serde_json::json!(r.score));

            if let Some(serde_json::Value::Object(map)) = &r.point.payload {
                for (k, v) in map {
                    row.insert(k.clone(), v.clone());
                }
            }
            row
        })
        .collect();

    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(QueryResult { rows, duration_ms })
}

fn contains_param_vector(condition: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::{Condition, VectorExpr};
    match condition {
        Condition::VectorSearch(vs) => matches!(vs.vector, VectorExpr::Parameter(_)),
        Condition::And(left, right) | Condition::Or(left, right) => {
            contains_param_vector(left) || contains_param_vector(right)
        }
        Condition::Group(inner) => contains_param_vector(inner),
        _ => false,
    }
}

#[cfg(test)]
fn contains_vector_search(condition: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::Condition;
    match condition {
        Condition::VectorSearch(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            contains_vector_search(left) || contains_vector_search(right)
        }
        Condition::Group(inner) => contains_vector_search(inner),
        _ => false,
    }
}

// NOTE: print_result and print_table moved to repl_output.rs (EPIC-061/US-004 refactoring)
/// Print query results in the specified format
pub fn print_result(result: &QueryResult, format: &str) {
    crate::repl_output::print_result(result, format);
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use velesdb_core::velesql::{
        CompareOp, Comparison, Condition, Value, VectorExpr, VectorSearch,
    };

    // =========================================================================
    // Tests for ReplConfig
    // =========================================================================

    #[test]
    fn test_repl_config_default() {
        let config = ReplConfig::default();
        assert!(config.timing);
        assert_eq!(config.format, OutputFormat::Table);
    }

    #[test]
    fn test_output_format_eq() {
        assert_eq!(OutputFormat::Table, OutputFormat::Table);
        assert_eq!(OutputFormat::Json, OutputFormat::Json);
        assert_ne!(OutputFormat::Table, OutputFormat::Json);
    }

    // =========================================================================
    // Tests for QueryResult
    // =========================================================================

    #[test]
    fn test_query_result_empty() {
        let result = QueryResult {
            rows: vec![],
            duration_ms: 0.0,
        };
        assert!(result.rows.is_empty());
        assert!((result.duration_ms - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_query_result_with_data() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), json!(1));
        row.insert("name".to_string(), json!("test"));

        let result = QueryResult {
            rows: vec![row],
            duration_ms: 1.5,
        };

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].get("id"), Some(&json!(1)));
        assert!((result.duration_ms - 1.5).abs() < f64::EPSILON);
    }

    // =========================================================================
    // Tests for contains_vector_search
    // =========================================================================

    #[test]
    fn test_contains_vector_search_with_vector() {
        let condition = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1, 0.2]),
        });
        assert!(contains_vector_search(&condition));
    }

    #[test]
    fn test_contains_vector_search_without_vector() {
        let condition = Condition::Comparison(Comparison {
            column: "category".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("tech".to_string()),
        });
        assert!(!contains_vector_search(&condition));
    }

    #[test]
    fn test_contains_vector_search_nested_and() {
        let vector_cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1]),
        });
        let other_cond = Condition::Comparison(Comparison {
            column: "x".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let combined = Condition::And(Box::new(other_cond), Box::new(vector_cond));
        assert!(contains_vector_search(&combined));
    }

    #[test]
    fn test_contains_vector_search_nested_or() {
        let vector_cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1]),
        });
        let other_cond = Condition::Comparison(Comparison {
            column: "x".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let combined = Condition::Or(Box::new(other_cond), Box::new(vector_cond));
        assert!(contains_vector_search(&combined));
    }

    #[test]
    fn test_contains_vector_search_group() {
        let vector_cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1]),
        });
        let grouped = Condition::Group(Box::new(vector_cond));
        assert!(contains_vector_search(&grouped));
    }

    #[test]
    fn test_contains_vector_search_no_match() {
        let cond_a = Condition::Comparison(Comparison {
            column: "a".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let cond_b = Condition::Comparison(Comparison {
            column: "b".to_string(),
            operator: CompareOp::Gt,
            value: Value::Integer(2),
        });
        let condition = Condition::And(Box::new(cond_a), Box::new(cond_b));
        assert!(!contains_vector_search(&condition));
    }

    // =========================================================================
    // Tests for print_result (output format logic)
    // =========================================================================

    #[test]
    fn test_print_result_empty() {
        let result = QueryResult {
            rows: vec![],
            duration_ms: 0.0,
        };
        // Should not panic on empty results
        print_result(&result, "table");
        print_result(&result, "json");
    }

    #[test]
    fn test_print_result_json_format() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), json!(1));

        let result = QueryResult {
            rows: vec![row],
            duration_ms: 1.0,
        };
        // Should not panic
        print_result(&result, "json");
        print_result(&result, "JSON"); // case insensitive
    }

    #[test]
    fn test_print_result_table_format() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), json!(1));
        row.insert("name".to_string(), json!("test"));

        let result = QueryResult {
            rows: vec![row],
            duration_ms: 1.0,
        };
        // Should not panic
        print_result(&result, "table");
        print_result(&result, "unknown"); // defaults to table
    }
}
