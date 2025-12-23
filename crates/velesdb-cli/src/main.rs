#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! `VelesDB` CLI - Interactive REPL for `VelesQL` queries
//!
//! Usage:
//!   `velesdb repl ./my_database`
//!   `velesdb query ./my_database "SELECT * FROM docs LIMIT 10"`
//!   `velesdb import ./data.jsonl --collection docs`

mod import;
mod repl;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use velesdb_core::DistanceMetric;

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

/// CLI metric option
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum MetricArg {
    #[default]
    Cosine,
    Euclidean,
    Dot,
}

impl From<MetricArg> for DistanceMetric {
    fn from(m: MetricArg) -> Self {
        match m {
            MetricArg::Cosine => DistanceMetric::Cosine,
            MetricArg::Euclidean => DistanceMetric::Euclidean,
            MetricArg::Dot => DistanceMetric::DotProduct,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Start interactive REPL
    Repl {
        /// Path to database directory
        #[arg(default_value = "./data")]
        path: PathBuf,
    },

    /// Execute a single query
    Query {
        /// Path to database directory
        path: PathBuf,

        /// `VelesQL` query to execute
        query: String,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Show database info
    Info {
        /// Path to database directory
        path: PathBuf,
    },

    /// Import vectors from CSV or JSONL file
    Import {
        /// Path to data file (CSV or JSONL)
        file: PathBuf,

        /// Path to database directory
        #[arg(short, long, default_value = "./data")]
        database: PathBuf,

        /// Collection name
        #[arg(short, long)]
        collection: String,

        /// Vector dimension (auto-detected if not specified)
        #[arg(long)]
        dimension: Option<usize>,

        /// Distance metric
        #[arg(long, value_enum, default_value = "cosine")]
        metric: MetricArg,

        /// ID column name (for CSV)
        #[arg(long, default_value = "id")]
        id_column: String,

        /// Vector column name (for CSV)
        #[arg(long, default_value = "vector")]
        vector_column: String,

        /// Batch size for insertion
        #[arg(long, default_value = "1000")]
        batch_size: usize,

        /// Show progress bar
        #[arg(long, default_value = "true")]
        progress: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Repl { path } => {
            repl::run(path)?;
        }
        Commands::Query {
            path,
            query,
            format,
        } => {
            let db = velesdb_core::Database::open(&path)?;
            let result = repl::execute_query(&db, &query)?;
            repl::print_result(&result, &format);
        }
        Commands::Info { path } => {
            let db = velesdb_core::Database::open(&path)?;
            println!("VelesDB Database: {}", path.display());
            println!("Collections:");
            for name in db.list_collections() {
                if let Some(col) = db.get_collection(&name) {
                    let config = col.config();
                    println!(
                        "  - {} ({} dims, {:?}, {} points)",
                        config.name, config.dimension, config.metric, config.point_count
                    );
                }
            }
        }
        Commands::Import {
            file,
            database,
            collection,
            dimension,
            metric,
            id_column,
            vector_column,
            batch_size,
            progress,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&database)?;
            let config = import::ImportConfig {
                collection,
                dimension,
                metric: metric.into(),
                batch_size,
                id_column,
                vector_column,
                show_progress: progress,
            };

            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");

            let stats = match ext.to_lowercase().as_str() {
                "jsonl" | "ndjson" => import::import_jsonl(&db, &file, &config)?,
                "csv" => import::import_csv(&db, &file, &config)?,
                _ => {
                    anyhow::bail!("Unsupported file format: {}. Use .csv or .jsonl", ext);
                }
            };

            println!("\n{}", "Import Summary".green().bold());
            println!("  Total records:    {}", stats.total);
            println!("  Imported:         {}", stats.imported.to_string().green());
            if stats.errors > 0 {
                println!("  Errors:           {}", stats.errors.to_string().red());
            }
            println!("  Duration:         {} ms", stats.duration_ms);
            println!(
                "  Throughput:       {:.0} records/sec",
                stats.records_per_sec()
            );
        }
    }

    Ok(())
}
