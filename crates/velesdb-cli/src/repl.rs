#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! REPL (Read-Eval-Print-Loop) for `VelesQL` queries

use anyhow::{Context, Result};
use colored::Colorize;
use comfy_table::{presets::UTF8_FULL, Cell, Color, ContentArrangement, Table};
use instant::Instant;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Completer, Editor, Helper, Highlighter, Hinter, Validator};
use std::collections::HashMap;
use std::path::PathBuf;
use velesdb_core::Database;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// REPL configuration
#[derive(Debug, Clone)]
pub struct ReplConfig {
    pub timing: bool,
    pub format: OutputFormat,
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            timing: true,
            format: OutputFormat::Table,
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

        _ => CommandResult::Error(format!("Unknown command: {cmd}")),
    }
}

fn print_help() {
    println!("\n{}", "VelesQL REPL Commands".bold().underline());
    println!();
    println!("  {}       Show this help", ".help".yellow());
    println!("  {}       Exit the REPL", ".quit".yellow());
    println!("  {} List all collections", ".collections".yellow());
    println!("  {} Show collection schema", ".schema <name>".yellow());
    println!("  {}   Toggle timing display", ".timing on|off".yellow());
    println!("  {}    Set output format", ".format table|json".yellow());
    println!("  {}      Clear screen", ".clear".yellow());
    println!();
    println!("{}", "VelesQL Examples:".bold().underline());
    println!();
    println!("  {}", "SELECT * FROM documents LIMIT 10;".italic().white());
    println!(
        "  {}",
        "SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, ...] LIMIT 5;"
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

    // For now, simple implementation - just list points with limit
    #[allow(clippy::cast_possible_truncation)]
    let limit = parsed.select.limit.unwrap_or(10) as usize;

    // Get all point IDs (simplified - in production would use index)
    let mut rows = Vec::new();

    // Check if there's a vector search in the where clause
    let has_vector_search = parsed
        .select
        .where_clause
        .as_ref()
        .is_some_and(contains_vector_search);

    if has_vector_search {
        // Vector search requires a vector parameter which we can't get from CLI directly
        // For demo, return empty with message
        println!(
            "{}",
            "Note: Vector search requires a query vector. Use REST API for vector queries."
                .yellow()
        );
    } else {
        // Just return first N points
        let ids: Vec<u64> = (1..=(limit as u64 * 2)).collect();
        let points = collection.get(&ids);

        for point in points.into_iter().flatten() {
            let mut row = HashMap::new();
            row.insert("id".to_string(), serde_json::json!(point.id));

            if let Some(serde_json::Value::Object(map)) = &point.payload {
                for (k, v) in map {
                    row.insert(k.clone(), v.clone());
                }
            }
            rows.push(row);

            if rows.len() >= limit {
                break;
            }
        }
    }

    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(QueryResult { rows, duration_ms })
}

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

/// Print query results in the specified format
pub fn print_result(result: &QueryResult, format: &str) {
    if result.rows.is_empty() {
        println!("{}", "No results.".dimmed());
        return;
    }

    match format.to_lowercase().as_str() {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&result.rows).unwrap());
        }
        _ => {
            print_table(&result.rows);
        }
    }
}

fn print_table(rows: &[HashMap<String, serde_json::Value>]) {
    if rows.is_empty() {
        return;
    }

    // Collect all column names
    let mut columns: Vec<String> = Vec::new();
    for row in rows {
        for key in row.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    columns.sort();

    // Ensure 'id' is first if present
    if let Some(pos) = columns.iter().position(|c| c == "id") {
        columns.remove(pos);
        columns.insert(0, "id".to_string());
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);

    // Header
    let header: Vec<Cell> = columns
        .iter()
        .map(|c| Cell::new(c).fg(Color::Cyan))
        .collect();
    table.set_header(header);

    // Rows
    for row in rows {
        let cells: Vec<Cell> = columns
            .iter()
            .map(|col| {
                let value = row.get(col).map_or("-".to_string(), |v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Null => "-".to_string(),
                    other => other.to_string(),
                });
                Cell::new(value)
            })
            .collect();
        table.add_row(cells);
    }

    println!("{table}");
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use velesdb_core::velesql::{
        CompareOp, Comparison, Condition, DistanceMetricType, Value, VectorExpr, VectorSearch,
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
        assert_eq!(result.duration_ms, 0.0);
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
        assert_eq!(result.duration_ms, 1.5);
    }

    // =========================================================================
    // Tests for contains_vector_search
    // =========================================================================

    #[test]
    fn test_contains_vector_search_with_vector() {
        let condition = Condition::VectorSearch(VectorSearch {
            metric: DistanceMetricType::Cosine,
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
            metric: DistanceMetricType::Cosine,
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
            metric: DistanceMetricType::Cosine,
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
            metric: DistanceMetricType::Cosine,
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
