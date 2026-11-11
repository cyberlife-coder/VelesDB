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

#[allow(clippy::pedantic)]
mod agent;
mod cli_types; // pedantic-clean ✓
#[allow(clippy::pedantic)]
mod collection_helpers;
#[allow(clippy::pedantic)]
mod commands;
mod export;
#[allow(clippy::pedantic)]
mod graph;
#[allow(clippy::pedantic)]
mod graph_display;
#[allow(clippy::pedantic)]
mod graph_doctor;
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
mod output;
#[allow(clippy::pedantic)]
mod repl;
mod repl_agent_cmds;
#[allow(clippy::pedantic)]
mod repl_cli_alias;
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
mod repl_meta_cmds;
#[allow(clippy::pedantic)]
mod repl_output;
#[allow(clippy::pedantic)]
mod repl_query_cmds;
#[allow(clippy::pedantic)]
mod repl_search_cmds;
#[allow(clippy::pedantic)]
mod session;

use clap::Parser;
use std::path::PathBuf;

use commands::{CollectionCommands, Commands, DataCommands, DbAction, QueryCommands};

#[derive(Parser)]
#[command(name = "velesdb")]
#[command(
    author,
    version,
    about = "VelesDB CLI - High-performance vector database"
)]
#[command(propagate_version = true, disable_help_subcommand = false)]
struct Cli {
    /// Path to a VelesDB TOML config file (search/HNSW/storage/limits/WAL
    /// batching). Applies to every command that opens a database, including
    /// the REPL. An invalid or missing path fails fast — never silently
    /// falls back to defaults. Without it, `./velesdb.toml` is read when it
    /// exists (an invalid one is an error), else the core defaults apply.
    #[arg(long = "config", global = true, env = "VELESDB_CONFIG")]
    config: Option<PathBuf>,

    /// Global output format (table, json, jsonl, csv); overrides the
    /// per-command `--format` of every command that prints data.
    #[arg(long = "output", global = true, value_name = "FORMAT")]
    output: Option<output::GlobalFormat>,

    /// Suppress human-readable status output; data still prints. For
    /// scripting/CI use together with `--output`.
    #[arg(long = "quiet", global = true)]
    quiet: bool,

    /// Disable colored output (same as setting NO_COLOR=1).
    #[arg(long = "no-color", global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Commands,
}

fn main() {
    if let Err(err) = cli_main() {
        eprintln!("Error: {err:#}");
        std::process::exit(output::exit_code(&err));
    }
}

/// Builds the server config overrides for `velesdb serve` from its flags.
///
/// `--config` is the CLI's global flag (also accepted as `VELESDB_CONFIG`),
/// forwarded to the server so the unified entrypoint reads the same file it
/// applies to every other command; API keys, log level and allowed hosts
/// resolve through the same env-var helpers `velesdb-server` itself uses.
#[cfg(feature = "serve")]
fn serve_overrides(
    data_dir: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    tls_cert: Option<String>,
    tls_key: Option<String>,
    rate_limit: Option<u32>,
    allowed_hosts: Option<Vec<String>>,
) -> velesdb_server::config::CliOverrides {
    velesdb_server::config::CliOverrides {
        config_path: helpers::config_path().map(std::path::Path::to_path_buf),
        data_dir,
        host,
        port,
        api_keys: velesdb_server::config::parse_api_keys_env(),
        tls_cert,
        tls_key,
        rate_limit,
        log_level: velesdb_server::logging::level_from_env(),
        allowed_hosts: velesdb_server::config::normalize_list_arg(allowed_hosts),
    }
}

fn cli_main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    output::init(output::OutputOptions {
        format: cli.output,
        quiet: cli.quiet,
        no_color: cli.no_color,
    });
    // Non-blocking update check (background thread, 2s timeout), sent once.
    // No data directory is known this early, so the install id lives in the
    // per-user config directory or, failing that, for this process only.
    // Disable: VELESDB_NO_UPDATE_CHECK=1 or [update_check] enabled=false in config.
    #[cfg(feature = "update-check")]
    {
        let (check_config, check_context) = update_check_plan(cli.config.as_deref());
        velesdb_core::spawn_update_check_with(check_config, check_context);
    }

    // Recorded once here; every command that opens a database (REPL and
    // one-shot alike) reads it back via `helpers::open_database`.
    helpers::set_config_path(cli.config);
    dispatch(cli.command)
}

/// What the CLI's update check runs with: the `[update_check]` section of the
/// config file given with `--config` / `VELESDB_CONFIG` (else `./velesdb.toml`),
/// and a context naming the binary `cli`. No data directory is known this
/// early, so the install id lives in the per-user config directory or, failing
/// that, for this process only.
#[cfg(feature = "update-check")]
fn update_check_plan(
    config_path: Option<&std::path::Path>,
) -> (
    velesdb_core::UpdateCheckConfig,
    velesdb_core::UpdateCheckContext,
) {
    (
        velesdb_core::UpdateCheckConfig::from_config_file(config_path),
        velesdb_core::UpdateCheckContext::new("cli", "core"),
    )
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
        Commands::Config { action } => handlers::handle_config(action),
        Commands::Db { action } => dispatch_db(action),
        #[cfg(feature = "serve")]
        Commands::Serve {
            data_dir,
            host,
            port,
            tls_cert,
            tls_key,
            rate_limit,
            allowed_hosts,
        } => velesdb_server::serve::run_with_overrides(serve_overrides(
            data_dir,
            host,
            port,
            tls_cert,
            tls_key,
            rate_limit,
            allowed_hosts,
        )),
        #[cfg(not(feature = "serve"))]
        Commands::Serve => anyhow::bail!(
            "`velesdb serve` needs the `serve` cargo feature; rebuild with              `--features serve` or use the standalone `velesdb-server` binary"
        ),
        Commands::Agent {
            path,
            question,
            format,
            max_iterations,
            save,
        } => run_agent(
            &path,
            &question.join(" "),
            &format,
            max_iterations,
            save.as_deref(),
        ),
    }
}

/// `velesdb agent`: run, print in `format`, optionally save the transcript.
fn run_agent(
    path: &std::path::Path,
    question: &str,
    format: &str,
    max: agent::MaxIterations,
    save: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let db = helpers::open_database(path)?;
    let report = agent::run(&db, question, max);
    let format = output::effective_format(format);
    if output::is_machine(&format) {
        output::print_machine(&agent::report_json(&report), &format)?;
    } else {
        agent::print_report(&report);
    }
    if let Some(file) = save {
        agent::save_report(&report, file)
            .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", file.display()))?;
    }
    Ok(())
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
            dimension,
            metric,
        } => handlers::handle_create_graph_collection(&path, &name, dimension, metric),
        CollectionCommands::CreateMetadata { path, name } => {
            handlers::handle_create_metadata_collection(&path, &name)
        }
        CollectionCommands::Delete { path, name, force } => {
            handlers::handle_delete_collection(&path, &name, force)
        }
        CollectionCommands::List { path, format } => {
            let format = output::effective_format(&format);
            handlers::handle_list(&path, &format)
        }
        CollectionCommands::Show {
            path,
            collection,
            samples,
            format,
        } => {
            let format = output::effective_format(&format);
            handlers::handle_show(&path, &collection, samples, &format)
        }
        CollectionCommands::Analyze {
            path,
            collection,
            format,
        } => {
            let format = output::effective_format(&format);
            handlers::handle_analyze(&path, &collection, &format)
        }
    }
}

/// Dispatches `data import`.
fn dispatch_data_import(args: commands::ImportArgs) -> anyhow::Result<()> {
    handlers::handle_import(
        &args.file,
        &args.database,
        args.collection,
        args.dimension,
        args.metric,
        args.storage_mode,
        args.id_column,
        args.vector_column,
        args.batch_size,
        args.progress,
        args.dry_run,
        args.on_conflict,
        args.continue_on_error,
        args.error_report,
    )
}

/// Dispatches data sub-commands.
fn dispatch_data(action: DataCommands) -> anyhow::Result<()> {
    match action {
        DataCommands::Import(args) => dispatch_data_import(args),
        DataCommands::Export {
            path,
            collection,
            output_file,
            format,
            where_clause,
            include_vectors,
        } => handlers::handle_export(
            &path,
            &collection,
            output_file,
            format,
            where_clause,
            include_vectors,
        ),
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
        } => {
            let format = output::effective_format(&format);
            handlers::handle_get(&path, &collection, id, &format)
        }
        DataCommands::DeletePoints {
            path,
            collection,
            ids,
        } => handlers::handle_delete_points(&path, &collection, &ids),
        DataCommands::Scroll {
            path,
            collection,
            batch_size,
            cursor,
            format,
        } => {
            let format = output::effective_format(&format);
            handlers::handle_scroll(&path, &collection, batch_size, cursor, &format)
        }
        DataCommands::StreamInsert {
            path,
            collection,
            batch_size,
        } => handlers::handle_stream_insert(&path, &collection, batch_size),
    }
}

/// Dispatches database-maintenance sub-commands.
fn dispatch_db(action: DbAction) -> anyhow::Result<()> {
    match action {
        DbAction::Stats { path, format } => handlers::handle_db_stats(&path, &format),
        DbAction::Vacuum {
            path,
            collection,
            compact,
        } => handlers::handle_db_vacuum(&path, collection.as_deref(), compact),
        DbAction::Snapshot { path, output_dir } => handlers::handle_db_snapshot(&path, &output_dir),
        DbAction::Restore { path, output_dir } => handlers::handle_db_restore(&path, &output_dir),
    }
}

/// Dispatches query sub-commands.
fn dispatch_query_cmd(action: QueryCommands) -> anyhow::Result<()> {
    match action {
        QueryCommands::Execute {
            path,
            query,
            collection,
            format,
        } => {
            let format = output::effective_format(&format);
            dispatch_query(&path, &query, collection.as_deref(), &format)
        }
        QueryCommands::Search {
            path,
            collection,
            vectors,
            top_k,
            strategy,
            rrf_k,
            branch_weights,
            format,
        } => {
            let format = output::effective_format(&format);
            handlers::handle_multi_search(
                &path,
                &collection,
                &vectors,
                top_k,
                &strategy,
                rrf_k,
                branch_weights.as_deref(),
                &format,
            )
        }
        QueryCommands::BatchSearch {
            path,
            collection,
            vectors,
            top_k,
            format,
        } => {
            let format = output::effective_format(&format);
            handlers::handle_batch_search(&path, &collection, &vectors, top_k, &format)
        }
        QueryCommands::Explain {
            path,
            query,
            format,
        } => {
            let format = output::effective_format(&format);
            handlers::handle_explain(&path, &query, &format)
        }
    }
}

/// Handles the `query` subcommand inline (trivial delegation).
///
/// `collection` targets queries without a `FROM` clause (e.g. a bare MATCH),
/// mirroring the REST `/query` collection field. One-shot `query execute` has no
/// REPL session, so no session settings are applied.
fn dispatch_query(
    path: &std::path::Path,
    query: &str,
    collection: Option<&str>,
    format: &str,
) -> anyhow::Result<()> {
    let db = helpers::open_database(path)?;
    let result = repl::execute_query(&db, query, collection, None)?;
    repl::print_result(&result, format);
    Ok(())
}

#[cfg(test)]
#[path = "output_parse_tests.rs"]
mod output_parse_tests;

#[cfg(test)]
#[path = "main_tests.rs"]
mod main_tests;

#[cfg(test)]
#[path = "repl_fixtures_tests.rs"]
mod test_fixtures;
