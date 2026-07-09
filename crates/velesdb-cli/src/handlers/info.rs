//! Handlers for read-only informational commands: `info`, `list`, `show`, `analyze`.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::collection_helpers;

/// Handles the `info` subcommand: prints database overview.
pub fn handle_info(path: &Path) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    println!("VelesDB Database: {}", path.display());
    println!("Collections:");
    for name in db.list_collections() {
        print_collection_summary(&db, &name);
    }
    Ok(())
}

/// Prints a single line summary for a collection in the `info` command.
fn print_collection_summary(db: &velesdb_core::Database, name: &str) {
    let type_label = collection_helpers::collection_type_label(db, name);
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let config = col.config();
            println!(
                "  - {} [{}] ({} dims, {:?}, {} points)",
                config.name, type_label, config.dimension, config.metric, config.point_count
            );
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            let edge_count = col.get_edges(None).len();
            println!("  - {} [{}] ({} edges)", col.name(), type_label, edge_count);
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            println!("  - {} [{}] ({} items)", col.name(), type_label, col.len());
        }
        None => {
            println!("  - {} [{}]", name, type_label);
        }
    }
}

/// Handles the `list` subcommand: lists all collections.
pub fn handle_list(path: &Path, format: &str) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let collections = db.list_collections();

    if format == "json" {
        print_list_json(&db, &collections)?;
    } else {
        print_list_table(&db, &collections);
    }
    Ok(())
}

/// Prints collection list as JSON.
fn print_list_json(db: &velesdb_core::Database, collections: &[String]) -> Result<()> {
    let data: Vec<_> = collections
        .iter()
        .map(|name| collection_to_json(db, name))
        .collect();
    println!("{}", serde_json::to_string_pretty(&data)?);
    Ok(())
}

/// Converts a single collection to a JSON value for listing.
fn collection_to_json(db: &velesdb_core::Database, name: &str) -> serde_json::Value {
    let type_label = collection_helpers::collection_type_label(db, name);
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let cfg = col.config();
            serde_json::json!({
                "name": cfg.name,
                "type": type_label,
                "dimension": cfg.dimension,
                "metric": format!("{:?}", cfg.metric),
                "point_count": cfg.point_count,
                "storage_mode": format!("{:?}", cfg.storage_mode)
            })
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            serde_json::json!({
                "name": col.name(),
                "type": type_label,
                "edge_count": col.get_edges(None).len(),
                "has_embeddings": col.has_embeddings()
            })
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            serde_json::json!({
                "name": col.name(),
                "type": type_label,
                "item_count": col.len()
            })
        }
        None => {
            serde_json::json!({
                "name": name,
                "type": type_label
            })
        }
    }
}

/// Prints collection list as a colored table.
fn print_list_table(db: &velesdb_core::Database, collections: &[String]) {
    println!("\n{}", "Collections".bold().underline());
    if collections.is_empty() {
        println!("  No collections found.\n");
        return;
    }
    for name in collections {
        print_collection_row(db, name);
    }
    println!("\n  Total: {} collection(s)\n", collections.len());
}

/// Prints a single colored row for the list table.
fn print_collection_row(db: &velesdb_core::Database, name: &str) {
    let type_label = collection_helpers::collection_type_label(db, name);
    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let cfg = col.config();
            println!(
                "  {} {} [{}] ({} dims, {:?}, {} points)",
                "\u{2022}".cyan(),
                cfg.name.green(),
                type_label.cyan(),
                cfg.dimension,
                cfg.metric,
                cfg.point_count
            );
        }
        Some(collection_helpers::TypedCollection::Graph(col)) => {
            println!(
                "  {} {} [{}] ({} edges)",
                "\u{2022}".cyan(),
                col.name().green(),
                type_label.cyan(),
                col.get_edges(None).len()
            );
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            println!(
                "  {} {} [{}] ({} items)",
                "\u{2022}".cyan(),
                col.name().green(),
                type_label.cyan(),
                col.len()
            );
        }
        None => {
            println!(
                "  {} {} [{}]",
                "\u{2022}".cyan(),
                name.green(),
                type_label.cyan()
            );
        }
    }
}

/// Handles the `show` subcommand: shows detailed info about one collection.
pub fn handle_show(path: &Path, collection: &str, samples: usize, format: &str) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let typed = collection_helpers::resolve_collection(&db, collection)
        .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;
    let type_label = collection_helpers::collection_type_label(&db, collection);

    match typed {
        collection_helpers::TypedCollection::Vector(col) => {
            show_vector(&col, type_label, samples, format)?;
        }
        collection_helpers::TypedCollection::Graph(col) => {
            show_graph(&col, type_label, format)?;
        }
        collection_helpers::TypedCollection::Metadata(col) => {
            show_metadata(&col, type_label, format)?;
        }
    }
    Ok(())
}

/// Displays vector collection details.
fn show_vector(
    col: &velesdb_core::VectorCollection,
    type_label: &str,
    samples: usize,
    format: &str,
) -> Result<()> {
    let cfg = col.config();
    if format == "json" {
        let data = serde_json::json!({
            "name": cfg.name,
            "type": type_label,
            "dimension": cfg.dimension,
            "metric": format!("{:?}", cfg.metric),
            "point_count": cfg.point_count,
            "storage_mode": format!("{:?}", cfg.storage_mode),
            "estimated_memory_mb": (cfg.point_count * cfg.dimension * 4) as f64 / 1_000_000.0
        });
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        print_vector_table(&cfg, type_label);
        if samples > 0 {
            print_vector_samples(col, samples);
        }
        println!();
    }
    Ok(())
}

/// Prints the vector collection details table (non-JSON).
fn print_vector_table(cfg: &velesdb_core::collection::CollectionConfig, type_label: &str) {
    println!("\n{}", "Collection Details".bold().underline());
    println!("  {} {}", "Name:".cyan(), cfg.name.green());
    println!("  {} {}", "Type:".cyan(), type_label.green());
    println!("  {} {}", "Dimension:".cyan(), cfg.dimension);
    println!("  {} {:?}", "Metric:".cyan(), cfg.metric);
    println!("  {} {}", "Point Count:".cyan(), cfg.point_count);
    println!("  {} {:?}", "Storage Mode:".cyan(), cfg.storage_mode);

    let estimated_mb = (cfg.point_count * cfg.dimension * 4) as f64 / 1_000_000.0;
    println!("  {} {:.2} MB", "Est. Memory:".cyan(), estimated_mb);
}

/// Prints sample vector records.
fn print_vector_samples(col: &velesdb_core::VectorCollection, samples: usize) {
    println!("\n{}", "Sample Records".bold().underline());
    // Sample from the collection's actual IDs, not an assumed contiguous
    // `1..=n` range: sparse/non-sequential IDs would otherwise show nothing.
    let ids: Vec<u64> = col.all_ids().into_iter().take(samples).collect();
    let points = col.get(&ids);
    for point in points.into_iter().flatten().take(samples) {
        println!("  ID: {}", point.id.to_string().green());
        if let Some(payload) = &point.payload {
            println!("    Payload: {}", payload);
        }
    }
}

/// Displays graph collection details.
fn show_graph(col: &velesdb_core::GraphCollection, type_label: &str, format: &str) -> Result<()> {
    let edge_count = col.get_edges(None).len();
    if format == "json" {
        let data = serde_json::json!({
            "name": col.name(),
            "type": type_label,
            "edge_count": edge_count,
            "has_embeddings": col.has_embeddings(),
            "schema": format!("{:?}", col.schema())
        });
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        println!("\n{}", "Collection Details".bold().underline());
        println!("  {} {}", "Name:".cyan(), col.name().green());
        println!("  {} {}", "Type:".cyan(), type_label.green());
        println!("  {} {}", "Edges:".cyan(), edge_count);
        println!("  {} {}", "Embeddings:".cyan(), col.has_embeddings());
        println!("  {} {:?}", "Schema:".cyan(), col.schema());
        println!();
    }
    Ok(())
}

/// Displays metadata collection details.
fn show_metadata(
    col: &velesdb_core::MetadataCollection,
    type_label: &str,
    format: &str,
) -> Result<()> {
    if format == "json" {
        let data = serde_json::json!({
            "name": col.name(),
            "type": type_label,
            "item_count": col.len()
        });
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        println!("\n{}", "Collection Details".bold().underline());
        println!("  {} {}", "Name:".cyan(), col.name().green());
        println!("  {} {}", "Type:".cyan(), type_label.green());
        println!("  {} {}", "Item Count:".cyan(), col.len());
        println!();
    }
    Ok(())
}

/// Handles the `analyze` subcommand: collection statistics.
pub fn handle_analyze(path: &Path, collection: &str, format: &str) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let stats = db
        .analyze_collection(collection)
        .map_err(|e| anyhow::anyhow!("Analyze error: {}", e))?;

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        print_analyze_table(collection, &stats);
    }
    Ok(())
}

/// Prints analysis statistics as a colored table.
fn print_analyze_table(collection: &str, stats: &velesdb_core::collection::stats::CollectionStats) {
    println!("\n{}", "Collection Statistics".bold().underline());
    println!("  {} {}", "Collection:".cyan(), collection.green());
    println!("  {} {}", "Total points:".cyan(), stats.total_points);
    println!("  {} {}", "Row count:".cyan(), stats.row_count);
    println!("  {} {}", "Deleted count:".cyan(), stats.deleted_count);
    println!("  {} {}", "Live rows:".cyan(), stats.live_row_count());
    println!(
        "  {} {:.2}%",
        "Deletion ratio:".cyan(),
        stats.deletion_ratio() * 100.0
    );
    println!(
        "  {} {} bytes",
        "Payload size:".cyan(),
        stats.payload_size_bytes
    );
    println!(
        "  {} {} bytes",
        "Avg row size:".cyan(),
        stats.avg_row_size_bytes
    );
    println!(
        "  {} {} bytes",
        "Total size:".cyan(),
        stats.total_size_bytes
    );

    print_analyze_indexes(stats);
    print_analyze_columns(stats);

    if let Some(ts) = stats.last_analyzed_epoch_ms {
        println!("\n  {} epoch {} ms", "Last analyzed:".cyan(), ts);
    }
    println!();
}

/// Prints index statistics if any exist.
fn print_analyze_indexes(stats: &velesdb_core::collection::stats::CollectionStats) {
    if stats.index_stats.is_empty() {
        return;
    }
    println!("\n  {}", "Indexes:".cyan().bold());
    for (name, idx) in &stats.index_stats {
        println!(
            "    {} ({}) \u{2014} {} entries, depth {}, {} bytes",
            name.green(),
            idx.index_type,
            idx.entry_count,
            idx.depth,
            idx.size_bytes
        );
    }
}

/// Prints column statistics if any exist.
fn print_analyze_columns(stats: &velesdb_core::collection::stats::CollectionStats) {
    if stats.column_stats.is_empty() {
        return;
    }
    println!("\n  {}", "Columns:".cyan().bold());
    for (name, col) in &stats.column_stats {
        println!(
            "    {} \u{2014} {} distinct, {} nulls",
            name.green(),
            col.distinct_count,
            col.null_count
        );
    }
}
