//! REPL output formatting module (EPIC-061/US-004 refactoring).
//!
//! Extracted from repl.rs to reduce file size and improve modularity.

use colored::Colorize;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use std::collections::HashMap;

use crate::repl::QueryResult;

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

/// Print results as a formatted table
pub fn print_table(rows: &[HashMap<String, serde_json::Value>]) {
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

/// Print REPL help message
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
