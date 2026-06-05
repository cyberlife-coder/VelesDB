//! Handlers for collection lifecycle commands: create, delete.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;
use velesdb_core::{DistanceMetric, StorageMode};

use crate::cli_types::{MetricArg, StorageModeArg};

/// Handles the `create-vector-collection` subcommand.
pub fn handle_create_vector_collection(
    path: &Path,
    name: &str,
    dimension: usize,
    metric: MetricArg,
    storage: StorageModeArg,
) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    db.create_vector_collection_with_options(name, dimension, metric.into(), storage.into())?;

    println!(
        "{} Vector collection '{}' created ({} dims, {:?}, {:?})",
        "\u{2705}".green(),
        name.cyan(),
        dimension,
        DistanceMetric::from(metric),
        StorageMode::from(storage),
    );
    Ok(())
}

/// Handles the `create-graph-collection` subcommand.
pub fn handle_create_graph_collection(path: &Path, name: &str) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let schema = velesdb_core::GraphSchema::schemaless();
    db.create_graph_collection(name, schema)?;

    println!(
        "{} Graph collection '{}' created (schemaless)",
        "\u{2705}".green(),
        name.cyan(),
    );
    Ok(())
}

/// Handles the `create-metadata-collection` subcommand.
pub fn handle_create_metadata_collection(path: &Path, name: &str) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    db.create_metadata_collection(name)?;

    println!(
        "{} Collection '{}' created (metadata-only)",
        "\u{2705}".green(),
        name.cyan()
    );
    Ok(())
}

/// Handles the `delete-collection` subcommand with optional confirmation.
pub fn handle_delete_collection(path: &Path, name: &str, force: bool) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;

    if !force {
        if !confirm_deletion(name)? {
            println!("{} Deletion cancelled.", "\u{2139}\u{fe0f}".cyan());
            return Ok(());
        }
    }

    db.delete_collection(name)?;
    println!(
        "{} Collection '{}' deleted.",
        "\u{2705}".green(),
        name.cyan()
    );
    Ok(())
}

/// Prompts the user to confirm collection deletion.
///
/// Returns `true` if the user typed "yes".
fn confirm_deletion(name: &str) -> Result<bool> {
    println!(
        "Are you sure you want to delete collection '{}'? This cannot be undone.",
        name.yellow()
    );
    print!("Type 'yes' to confirm: ");
    use std::io::Write;
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim() == "yes")
}
