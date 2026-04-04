// CLI - pedantic lints reactivated per-module; nursery globally relaxed
#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
//! `VelesDB` CLI - Interactive REPL for `VelesQL` queries
//!
//! Usage:
//!   `velesdb repl ./my_database`
//!   `velesdb query ./my_database "SELECT * FROM docs LIMIT 10"`
//!   `velesdb import ./data.jsonl --collection docs`

mod cli_types; // pedantic-clean ✓
#[allow(clippy::pedantic)]
mod collection_helpers;
#[allow(clippy::pedantic)]
mod commands;
#[allow(clippy::pedantic)]
mod graph;
#[allow(clippy::pedantic)]
mod graph_display;
#[allow(clippy::pedantic)]
mod graph_handlers;
#[allow(clippy::pedantic)]
mod handlers;
#[allow(clippy::pedantic)]
mod helpers;
#[allow(clippy::pedantic)]
mod import;
#[allow(clippy::pedantic)]
mod license;
#[allow(clippy::pedantic)]
mod repl;
#[allow(clippy::pedantic)]
mod repl_collection_cmds;
#[allow(clippy::pedantic)]
mod repl_commands;
#[allow(clippy::pedantic)]
mod repl_config_cmds;
#[allow(clippy::pedantic)]
mod repl_data_cmds;
#[allow(clippy::pedantic)]
mod repl_execute;
#[allow(clippy::pedantic)]
mod repl_graph_cmds;
#[allow(clippy::pedantic)]
mod repl_output;
#[allow(clippy::pedantic)]
mod repl_query_cmds;
#[allow(clippy::pedantic)]
mod repl_search_cmds;
#[allow(clippy::pedantic)]
mod session;

use clap::Parser;

use commands::{CollectionCommands, Commands, DataCommands, QueryCommands};

#[derive(Parser)]
#[command(name = "velesdb")]
#[command(
    author,
    version,
    about = "VelesDB CLI - High-performance vector database"
)]
#[command(propagate_version = true, disable_help_subcommand = false)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn main() -> anyhow::Result<()> {
    cli_main()
}

fn cli_main() -> anyhow::Result<()> {
    // Non-blocking update check (background thread, 2s timeout).
    // Disable: VELESDB_NO_UPDATE_CHECK=1 or [update_check] enabled=false in config.
    #[cfg(feature = "update-check")]
    velesdb_core::spawn_update_check(
        velesdb_core::UpdateCheckConfig::default(),
        std::path::PathBuf::from("."),
        "core".to_string(),
    );

    let cli = Cli::parse();
    dispatch(cli.command)
}

/// Dispatches a parsed CLI command to its handler.
fn dispatch(command: Commands) -> anyhow::Result<()> {
    match command {
        // Standalone commands
        Commands::Repl { path } => repl::run(path),
        Commands::Info { path } => handlers::handle_info(&path),
        Commands::Completions { shell } => {
            handlers::handle_completions::<Cli>(shell);
            Ok(())
        }
        Commands::Simd { action } => {
            handlers::handle_simd(action);
            Ok(())
        }
        Commands::License { action } => handlers::handle_license(action),

        // Grouped sub-commands
        Commands::Collection { action } => dispatch_collection(action),
        Commands::Data { action } => dispatch_data(action),
        Commands::QueryCmd { action } => dispatch_query_cmd(action),
        Commands::Graph { action } => graph::handle(action),
        Commands::Index { action } => handlers::handle_index(action),
    }
}

/// Dispatches collection sub-commands.
fn dispatch_collection(action: CollectionCommands) -> anyhow::Result<()> {
    match action {
        CollectionCommands::CreateVector {
            path,
            name,
            dimension,
            metric,
            storage,
        } => handlers::handle_create_vector_collection(&path, &name, dimension, metric, storage),
        CollectionCommands::CreateGraph {
            path,
            name,
            schemaless,
        } => handlers::handle_create_graph_collection(&path, &name, schemaless),
        CollectionCommands::CreateMetadata { path, name } => {
            handlers::handle_create_metadata_collection(&path, &name)
        }
        CollectionCommands::Delete { path, name, force } => {
            handlers::handle_delete_collection(&path, &name, force)
        }
        CollectionCommands::List { path, format } => handlers::handle_list(&path, &format),
        CollectionCommands::Show {
            path,
            collection,
            samples,
            format,
        } => handlers::handle_show(&path, &collection, samples, &format),
        CollectionCommands::Analyze {
            path,
            collection,
            format,
        } => handlers::handle_analyze(&path, &collection, &format),
    }
}

/// Dispatches data sub-commands.
fn dispatch_data(action: DataCommands) -> anyhow::Result<()> {
    match action {
        DataCommands::Import {
            file,
            database,
            collection,
            dimension,
            metric,
            storage_mode,
            id_column,
            vector_column,
            batch_size,
            progress,
        } => handlers::handle_import(
            &file,
            &database,
            collection,
            dimension,
            metric,
            storage_mode,
            id_column,
            vector_column,
            batch_size,
            progress,
        ),
        DataCommands::Export {
            path,
            collection,
            output,
            include_vectors,
        } => handlers::handle_export(&path, &collection, output, include_vectors),
        DataCommands::Upsert {
            path,
            collection,
            id,
            vector,
            payload,
        } => handlers::handle_upsert(&path, &collection, id, vector, payload),
        DataCommands::Get {
            path,
            collection,
            id,
            format,
        } => handlers::handle_get(&path, &collection, id, &format),
        DataCommands::DeletePoints {
            path,
            collection,
            ids,
        } => handlers::handle_delete_points(&path, &collection, &ids),
    }
}

/// Dispatches query sub-commands.
fn dispatch_query_cmd(action: QueryCommands) -> anyhow::Result<()> {
    match action {
        QueryCommands::Execute {
            path,
            query,
            format,
        } => dispatch_query(&path, &query, &format),
        QueryCommands::Search {
            path,
            collection,
            vectors,
            top_k,
            strategy,
            rrf_k,
            format,
        } => handlers::handle_multi_search(
            &path,
            &collection,
            &vectors,
            top_k,
            &strategy,
            rrf_k,
            &format,
        ),
        QueryCommands::Explain {
            path,
            query,
            format,
        } => handlers::handle_explain(&path, &query, &format),
    }
}

/// Handles the `query` subcommand inline (trivial delegation).
fn dispatch_query(path: &std::path::Path, query: &str, format: &str) -> anyhow::Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let result = repl::execute_query(&db, query, None)?;
    repl::print_result(&result, format);
    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cli_types::{MetricArg, StorageModeArg};
    use velesdb_core::{DistanceMetric, StorageMode};

    #[test]
    fn test_commands_enum_size_below_threshold() {
        let size = std::mem::size_of::<Commands>();
        eprintln!("Commands enum size: {} bytes", size);
        // Sub-enum grouping should keep the enum well under 1 KB.
        assert!(
            size < 1024,
            "Commands enum is {} bytes, expected < 1024",
            size
        );
    }

    // =========================================================================
    // Tests for MetricArg conversions
    // =========================================================================

    #[test]
    fn test_metric_arg_cosine() {
        let metric: DistanceMetric = MetricArg::Cosine.into();
        assert_eq!(metric, DistanceMetric::Cosine);
    }

    #[test]
    fn test_metric_arg_euclidean() {
        let metric: DistanceMetric = MetricArg::Euclidean.into();
        assert_eq!(metric, DistanceMetric::Euclidean);
    }

    #[test]
    fn test_metric_arg_dot() {
        let metric: DistanceMetric = MetricArg::Dot.into();
        assert_eq!(metric, DistanceMetric::DotProduct);
    }

    #[test]
    fn test_metric_arg_hamming() {
        let metric: DistanceMetric = MetricArg::Hamming.into();
        assert_eq!(metric, DistanceMetric::Hamming);
    }

    #[test]
    fn test_metric_arg_jaccard() {
        let metric: DistanceMetric = MetricArg::Jaccard.into();
        assert_eq!(metric, DistanceMetric::Jaccard);
    }

    // =========================================================================
    // Tests for StorageModeArg conversions (Phase 1.2)
    // =========================================================================

    #[test]
    fn test_storage_mode_arg_full() {
        let mode: StorageMode = StorageModeArg::Full.into();
        assert_eq!(mode, StorageMode::Full);
    }

    #[test]
    fn test_storage_mode_arg_sq8() {
        let mode: StorageMode = StorageModeArg::Sq8.into();
        assert_eq!(mode, StorageMode::SQ8);
    }

    #[test]
    fn test_storage_mode_arg_binary() {
        let mode: StorageMode = StorageModeArg::Binary.into();
        assert_eq!(mode, StorageMode::Binary);
    }

    #[test]
    fn test_storage_mode_arg_pq() {
        let mode: StorageMode = StorageModeArg::Pq.into();
        assert_eq!(mode, StorageMode::ProductQuantization);
    }

    #[test]
    fn test_storage_mode_arg_rabitq() {
        let mode: StorageMode = StorageModeArg::Rabitq.into();
        assert_eq!(mode, StorageMode::RaBitQ);
    }

    #[test]
    fn test_storage_mode_arg_default_is_full() {
        let mode = StorageModeArg::default();
        assert!(matches!(mode, StorageModeArg::Full));
    }
}
