// CLI - pedantic/nursery lints relaxed
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
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

mod cli_types;
mod collection_helpers;
mod commands;
mod graph;
mod graph_display;
mod graph_handlers;
mod handlers;
mod helpers;
mod import;
mod license;
mod repl;
mod repl_collection_cmds;
mod repl_commands;
mod repl_config_cmds;
mod repl_data_cmds;
mod repl_graph_cmds;
mod repl_output;
mod repl_query_cmds;
mod repl_search_cmds;
mod session;

use clap::Parser;

use commands::Commands;

#[derive(Parser)]
#[command(name = "velesdb")]
#[command(
    author,
    version,
    about = "VelesDB CLI - High-performance vector database"
)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn main() -> anyhow::Result<()> {
    // Clap's derive-macro help generation for our 22-variant Commands enum
    // exceeds the default ~2 MB stack in unoptimized (debug/test) builds.
    // Spawn the real entry point on an 8 MB stack to accommodate this.
    const STACK_SIZE: usize = 8 * 1024 * 1024;
    let handler = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(cli_main)?;
    match handler.join() {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
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
        Commands::Repl { path } => repl::run(path),
        Commands::Query {
            path,
            query,
            format,
        } => dispatch_query(&path, &query, &format),
        Commands::Info { path } => handlers::handle_info(&path),
        Commands::List { path, format } => handlers::handle_list(&path, &format),
        Commands::Show {
            path,
            collection,
            samples,
            format,
        } => handlers::handle_show(&path, &collection, samples, &format),
        Commands::Export {
            path,
            collection,
            output,
            include_vectors,
        } => handlers::handle_export(&path, &collection, output, include_vectors),
        Commands::Import {
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
        Commands::License { action } => handlers::handle_license(action),
        Commands::MultiSearch {
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
        Commands::CreateMetadataCollection { path, name } => {
            handlers::handle_create_metadata_collection(&path, &name)
        }
        Commands::Get {
            path,
            collection,
            id,
            format,
        } => handlers::handle_get(&path, &collection, id, &format),
        Commands::Graph { action } => graph::handle(action),
        Commands::Completions { shell } => {
            handlers::handle_completions::<Cli>(shell);
            Ok(())
        }
        Commands::Simd { action } => {
            handlers::handle_simd(action);
            Ok(())
        }
        Commands::CreateVectorCollection {
            path,
            name,
            dimension,
            metric,
            storage,
        } => handlers::handle_create_vector_collection(&path, &name, dimension, metric, storage),
        Commands::CreateGraphCollection {
            path,
            name,
            schemaless,
        } => handlers::handle_create_graph_collection(&path, &name, schemaless),
        Commands::DeleteCollection { path, name, force } => {
            handlers::handle_delete_collection(&path, &name, force)
        }
        Commands::Explain {
            path,
            query,
            format,
        } => handlers::handle_explain(&path, &query, &format),
        Commands::Analyze {
            path,
            collection,
            format,
        } => handlers::handle_analyze(&path, &collection, &format),
        Commands::DeletePoints {
            path,
            collection,
            ids,
        } => handlers::handle_delete_points(&path, &collection, &ids),
        Commands::Upsert {
            path,
            collection,
            id,
            vector,
            payload,
        } => handlers::handle_upsert(&path, &collection, id, vector, payload),
        Commands::Index { action } => handlers::handle_index(action),
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
