//! Handler for the `index` subcommand: create, drop, list indexes.

use anyhow::Result;
use colored::Colorize;

use crate::cli_types::IndexTypeArg;
use crate::commands::IndexAction;

/// Handles the `index` subcommand: dispatches to create/drop/list.
pub fn handle_index(action: IndexAction) -> Result<()> {
    match action {
        IndexAction::Create {
            path,
            collection,
            field,
            index_type,
            label,
        } => handle_index_create(&path, &collection, &field, index_type, label.as_deref()),
        IndexAction::Drop {
            path,
            collection,
            label,
            property,
        } => handle_index_drop(&path, &collection, &label, &property),
        IndexAction::List {
            path,
            collection,
            format,
        } => handle_index_list(&path, &collection, &format),
    }
}

/// Creates an index on a collection field.
fn handle_index_create(
    path: &std::path::Path,
    collection: &str,
    field: &str,
    index_type: IndexTypeArg,
    label: Option<&str>,
) -> Result<()> {
    let db = crate::helpers::open_database(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

    match index_type {
        IndexTypeArg::Secondary => {
            col.create_index(field)
                .map_err(|e| anyhow::anyhow!("Create index failed: {e}"))?;
            println!(
                "{} Secondary index created on field '{}' in '{}'",
                "\u{2705}".green(),
                field.cyan(),
                collection.cyan()
            );
        }
        IndexTypeArg::Property => {
            let lbl = label.unwrap_or("default");
            col.create_property_index(lbl, field)
                .map_err(|e| anyhow::anyhow!("Create index failed: {e}"))?;
            println!(
                "{} Property index created on '{}:{}' in '{}'",
                "\u{2705}".green(),
                lbl.cyan(),
                field.cyan(),
                collection.cyan()
            );
        }
        IndexTypeArg::Range => {
            let lbl = label.unwrap_or("default");
            col.create_range_index(lbl, field)
                .map_err(|e| anyhow::anyhow!("Create index failed: {e}"))?;
            println!(
                "{} Range index created on '{}:{}' in '{}'",
                "\u{2705}".green(),
                lbl.cyan(),
                field.cyan(),
                collection.cyan()
            );
        }
    }
    Ok(())
}

/// Drops an index from a collection.
fn handle_index_drop(
    path: &std::path::Path,
    collection: &str,
    label: &str,
    property: &str,
) -> Result<()> {
    let db = crate::helpers::open_database(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

    let dropped = col
        .drop_index(label, property)
        .map_err(|e| anyhow::anyhow!("Drop index failed: {e}"))?;

    if dropped {
        println!(
            "{} Index '{}:{}' dropped from '{}'",
            "\u{2705}".green(),
            label.cyan(),
            property.cyan(),
            collection.cyan()
        );
    } else {
        println!(
            "{} No index '{}:{}' found in '{}'",
            "\u{26a0}\u{fe0f}".yellow(),
            label,
            property,
            collection
        );
    }
    Ok(())
}

/// Lists all indexes on a collection.
fn handle_index_list(path: &std::path::Path, collection: &str, format: &str) -> Result<()> {
    let db = crate::helpers::open_database(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

    let indexes = col.list_indexes();

    if format == "json" {
        print_indexes_json(&indexes)?;
    } else {
        print_indexes_table(&indexes);
    }
    Ok(())
}

/// Prints indexes as JSON.
fn print_indexes_json(indexes: &[velesdb_core::IndexInfo]) -> Result<()> {
    let data: Vec<_> = indexes
        .iter()
        .map(|idx| {
            serde_json::json!({
                "label": idx.label,
                "property": idx.property,
                "index_type": idx.index_type,
                "cardinality": idx.cardinality,
                "memory_bytes": idx.memory_bytes
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&data)?);
    Ok(())
}

/// Prints indexes as a colored table.
fn print_indexes_table(indexes: &[velesdb_core::IndexInfo]) {
    println!("\n{}", "Indexes".bold().underline());
    if indexes.is_empty() {
        println!("  No indexes found.\n");
        return;
    }
    for idx in indexes {
        println!(
            "  {} {}:{} ({}) \u{2014} {} unique values, {} bytes",
            "\u{2022}".cyan(),
            idx.label.green(),
            idx.property.green(),
            idx.index_type,
            idx.cardinality,
            idx.memory_bytes
        );
    }
    println!("\n  Total: {} index(es)\n", indexes.len());
}
