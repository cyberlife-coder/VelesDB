//! REPL commands for query analysis and index management.
//!
//! Covers: `.explain`, `.analyze`, `.indexes`, `.delete`, `.flush`,
//! `.create-index`, `.drop-index`.

use colored::Colorize;
use velesdb_core::Database;

use crate::repl_commands::{parse_flag, CommandResult};

pub(crate) fn cmd_explain(db: &Database, parts: &[&str]) -> CommandResult {
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

pub(crate) fn cmd_analyze(db: &Database, parts: &[&str]) -> CommandResult {
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

pub(crate) fn cmd_indexes(db: &Database, parts: &[&str]) -> CommandResult {
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

pub(crate) fn cmd_delete(db: &Database, parts: &[&str]) -> CommandResult {
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

pub(crate) fn cmd_flush(db: &Database, parts: &[&str]) -> CommandResult {
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

pub(crate) fn cmd_create_index(db: &Database, parts: &[&str]) -> CommandResult {
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

pub(crate) fn cmd_drop_index(db: &Database, parts: &[&str]) -> CommandResult {
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
