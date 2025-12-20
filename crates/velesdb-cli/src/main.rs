#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! `VelesDB` CLI - Interactive REPL for `VelesQL` queries
//!
//! Usage:
//!   `velesdb repl ./my_database`
//!   `velesdb query ./my_database "SELECT * FROM docs LIMIT 10"`

mod repl;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    }

    Ok(())
}
