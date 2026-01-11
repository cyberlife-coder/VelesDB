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

mod import;
mod license;
mod repl;
mod session;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use velesdb_core::{DistanceMetric, StorageMode};

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
    Hamming,
    Jaccard,
}

impl From<MetricArg> for DistanceMetric {
    fn from(m: MetricArg) -> Self {
        match m {
            MetricArg::Cosine => DistanceMetric::Cosine,
            MetricArg::Euclidean => DistanceMetric::Euclidean,
            MetricArg::Dot => DistanceMetric::DotProduct,
            MetricArg::Hamming => DistanceMetric::Hamming,
            MetricArg::Jaccard => DistanceMetric::Jaccard,
        }
    }
}

/// CLI storage mode option
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum StorageModeArg {
    #[default]
    Full,
    Sq8,
    Binary,
}

impl From<StorageModeArg> for StorageMode {
    fn from(m: StorageModeArg) -> Self {
        match m {
            StorageModeArg::Full => StorageMode::Full,
            StorageModeArg::Sq8 => StorageMode::SQ8,
            StorageModeArg::Binary => StorageMode::Binary,
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

    /// List all collections in the database
    List {
        /// Path to database directory
        path: PathBuf,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Show detailed information about a collection
    Show {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Show sample records
        #[arg(short, long, default_value = "0")]
        samples: usize,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Export a collection to JSON file
    Export {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Include vectors in export
        #[arg(long, default_value = "true")]
        include_vectors: bool,
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

        /// Storage mode (full, sq8, binary)
        #[arg(long, value_enum, default_value = "full")]
        storage_mode: StorageModeArg,

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

    /// License management commands
    License {
        #[command(subcommand)]
        action: LicenseAction,
    },

    /// Create a metadata-only collection (no vectors)
    CreateMetadataCollection {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,
    },

    /// Get a point by ID
    Get {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Point ID to retrieve
        id: u64,

        /// Output format (table, json)
        #[arg(short, long, default_value = "json")]
        format: String,
    },

    /// Perform multi-query search with fusion
    MultiSearch {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Query vectors as JSON array of arrays (e.g., '[[1.0, 0.0], [0.0, 1.0]]')
        vectors: String,

        /// Number of results to return
        #[arg(short = 'k', long, default_value = "10")]
        top_k: usize,

        /// Fusion strategy (average, maximum, rrf, weighted)
        #[arg(short, long, default_value = "rrf")]
        strategy: String,

        /// RRF k parameter (only for rrf strategy)
        #[arg(long, default_value = "60")]
        rrf_k: u32,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },
}

#[derive(Subcommand)]
enum LicenseAction {
    /// Show current license status
    Show,

    /// Activate a license key
    Activate {
        /// License key from email (format: base64_payload.base64_signature)
        key: String,
    },

    /// Verify a license key without activating it
    Verify {
        /// License key to verify
        key: String,

        /// Public key for verification (base64 encoded)
        #[arg(short, long)]
        public_key: String,
    },
}

#[allow(clippy::too_many_lines)]
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
        Commands::List { path, format } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let collections = db.list_collections();

            if format == "json" {
                let data: Vec<_> = collections
                    .iter()
                    .filter_map(|name| db.get_collection(name))
                    .map(|col| {
                        let cfg = col.config();
                        serde_json::json!({
                            "name": cfg.name,
                            "dimension": cfg.dimension,
                            "metric": format!("{:?}", cfg.metric),
                            "point_count": cfg.point_count,
                            "storage_mode": format!("{:?}", cfg.storage_mode)
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                println!("\n{}", "Collections".bold().underline());
                if collections.is_empty() {
                    println!("  No collections found.\n");
                } else {
                    for name in &collections {
                        if let Some(col) = db.get_collection(name) {
                            let cfg = col.config();
                            println!(
                                "  {} {} ({} dims, {:?}, {} points)",
                                "•".cyan(),
                                cfg.name.green(),
                                cfg.dimension,
                                cfg.metric,
                                cfg.point_count
                            );
                        }
                    }
                    println!("\n  Total: {} collection(s)\n", collections.len());
                }
            }
        }
        Commands::Show {
            path,
            collection,
            samples,
            format,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let col = db
                .get_collection(&collection)
                .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;

            let cfg = col.config();

            if format == "json" {
                let data = serde_json::json!({
                    "name": cfg.name,
                    "dimension": cfg.dimension,
                    "metric": format!("{:?}", cfg.metric),
                    "point_count": cfg.point_count,
                    "storage_mode": format!("{:?}", cfg.storage_mode),
                    "estimated_memory_mb": (cfg.point_count * cfg.dimension * 4) as f64 / 1_000_000.0
                });
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                println!("\n{}", "Collection Details".bold().underline());
                println!("  {} {}", "Name:".cyan(), cfg.name.green());
                println!("  {} {}", "Dimension:".cyan(), cfg.dimension);
                println!("  {} {:?}", "Metric:".cyan(), cfg.metric);
                println!("  {} {}", "Point Count:".cyan(), cfg.point_count);
                println!("  {} {:?}", "Storage Mode:".cyan(), cfg.storage_mode);

                let estimated_mb = (cfg.point_count * cfg.dimension * 4) as f64 / 1_000_000.0;
                println!("  {} {:.2} MB", "Est. Memory:".cyan(), estimated_mb);

                if samples > 0 {
                    println!("\n{}", "Sample Records".bold().underline());
                    let ids: Vec<u64> = (1..=(samples as u64 * 2)).collect();
                    let points = col.get(&ids);

                    for point in points.into_iter().flatten().take(samples) {
                        println!("  ID: {}", point.id.to_string().green());
                        if let Some(payload) = &point.payload {
                            println!("    Payload: {}", payload);
                        }
                    }
                }
                println!();
            }
        }
        Commands::Export {
            path,
            collection,
            output,
            include_vectors,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let col = db
                .get_collection(&collection)
                .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;

            let cfg = col.config();
            let output_path =
                output.unwrap_or_else(|| PathBuf::from(format!("{}.json", collection)));

            println!(
                "Exporting {} records from {}...",
                cfg.point_count,
                collection.green()
            );

            let mut records = Vec::new();
            let batch_size = 1000;

            for batch_start in (0..cfg.point_count).step_by(batch_size) {
                let ids: Vec<u64> =
                    ((batch_start as u64 + 1)..=((batch_start + batch_size) as u64)).collect();
                let points = col.get(&ids);

                for point in points.into_iter().flatten() {
                    let mut record = serde_json::Map::new();
                    record.insert("id".to_string(), serde_json::json!(point.id));
                    if include_vectors {
                        record.insert("vector".to_string(), serde_json::json!(point.vector));
                    }
                    if let Some(payload) = &point.payload {
                        record.insert("payload".to_string(), payload.clone());
                    }
                    records.push(serde_json::Value::Object(record));
                }
            }

            std::fs::write(&output_path, serde_json::to_string_pretty(&records)?)?;
            println!(
                "{} Exported {} records to {}",
                "✓".green(),
                records.len(),
                output_path.display().to_string().green()
            );
        }
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
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&database)?;
            let config = import::ImportConfig {
                collection,
                dimension,
                metric: metric.into(),
                storage_mode: storage_mode.into(),
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
        Commands::License { action } => {
            use colored::Colorize;

            match action {
                LicenseAction::Show => {
                    if let Ok(key) = license::load_license_key() {
                        // Try to get public key from environment
                        let public_key = std::env::var("VELESDB_LICENSE_PUBLIC_KEY")
                            .unwrap_or_else(|_| {
                                println!("{}", "⚠️  Warning: VELESDB_LICENSE_PUBLIC_KEY not set in environment".yellow());
                                println!("   Set it with: export VELESDB_LICENSE_PUBLIC_KEY=<base64_key>");
                                println!("   Using embedded development key for validation...\n");
                                // Development fallback key (same as velesdb-premium)
                                "MCowBQYDK2VwAyEADevelopmentKeyReplaceMeInProd==".to_string()
                            });

                        match license::validate_license(&key, &public_key) {
                            Ok(info) => {
                                license::display_license_info(&info);
                            }
                            Err(e) => {
                                println!("{} {}", "❌ License validation failed:".red().bold(), e);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        println!("{}", "❌ No license found".red().bold());
                        println!("\n{}", "To activate a license:".cyan());
                        println!("  velesdb license activate <license_key>");
                        println!("\n{}", "License keys are stored in:".cyan());
                        if let Ok(path) = license::get_license_config_path() {
                            println!("  {}", path.display());
                        }
                        std::process::exit(1);
                    }
                }
                LicenseAction::Activate { key } => {
                    // Try to get public key from environment
                    let public_key =
                        std::env::var("VELESDB_LICENSE_PUBLIC_KEY").unwrap_or_else(|_| {
                            println!(
                                "{}",
                                "⚠️  Warning: VELESDB_LICENSE_PUBLIC_KEY not set in environment"
                                    .yellow()
                            );
                            println!(
                                "   Set it with: export VELESDB_LICENSE_PUBLIC_KEY=<base64_key>"
                            );
                            println!("   Using embedded development key for validation...\n");
                            "MCowBQYDK2VwAyEADevelopmentKeyReplaceMeInProd==".to_string()
                        });

                    // Validate the license first
                    match license::validate_license(&key, &public_key) {
                        Ok(info) => {
                            // Save the license
                            license::save_license_key(&key)?;

                            println!("{}", "✅ License activated successfully!".green().bold());
                            println!();
                            license::display_license_info(&info);

                            if let Ok(path) = license::get_license_config_path() {
                                println!("{}", "License saved to:".cyan());
                                println!("  {}", path.display());
                            }
                        }
                        Err(e) => {
                            println!("{} {}", "❌ License activation failed:".red().bold(), e);
                            println!("\n{}", "Please check:".yellow());
                            println!("  • License key format is correct (base64_payload.base64_signature)");
                            println!("  • License has not expired");
                            println!(
                                "  • Public key is correctly set in VELESDB_LICENSE_PUBLIC_KEY"
                            );
                            std::process::exit(1);
                        }
                    }
                }
                LicenseAction::Verify { key, public_key } => {
                    match license::validate_license(&key, &public_key) {
                        Ok(info) => {
                            println!("{}", "✅ License is VALID".green().bold());
                            println!();
                            license::display_license_info(&info);
                        }
                        Err(e) => {
                            println!("{} {}", "❌ License verification failed:".red().bold(), e);
                            println!("\n{}", "Possible reasons:".yellow());
                            println!("  • Invalid signature (license may have been tampered with)");
                            println!("  • Wrong public key");
                            println!("  • License has expired");
                            println!("  • Malformed license format");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::MultiSearch {
            path,
            collection,
            vectors,
            top_k,
            strategy,
            rrf_k,
            format,
        } => {
            use colored::Colorize;
            use velesdb_core::FusionStrategy;

            let db = velesdb_core::Database::open(&path)?;
            let col = db
                .get_collection(&collection)
                .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;

            // Parse vectors from JSON
            let parsed_vectors: Vec<Vec<f32>> = serde_json::from_str(&vectors)
                .map_err(|e| anyhow::anyhow!("Invalid vectors JSON: {}", e))?;

            if parsed_vectors.is_empty() {
                anyhow::bail!("At least one query vector is required");
            }

            // Parse fusion strategy
            let fusion_strategy = match strategy.to_lowercase().as_str() {
                "average" | "avg" => FusionStrategy::Average,
                "maximum" | "max" => FusionStrategy::Maximum,
                "rrf" => FusionStrategy::RRF { k: rrf_k },
                "weighted" => FusionStrategy::Weighted {
                    avg_weight: 0.5,
                    max_weight: 0.3,
                    hit_weight: 0.2,
                },
                _ => anyhow::bail!(
                    "Invalid strategy '{}'. Valid: average, maximum, rrf, weighted",
                    strategy
                ),
            };

            // Convert to slices
            let query_refs: Vec<&[f32]> = parsed_vectors.iter().map(|v| v.as_slice()).collect();

            // Execute multi-query search
            let results = col
                .multi_query_search(&query_refs, top_k, fusion_strategy, None)
                .map_err(|e| anyhow::anyhow!("Search failed: {}", e))?;

            if format == "json" {
                let output: Vec<_> = results
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": r.point.id,
                            "score": r.score,
                            "payload": r.point.payload
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "\n{} (strategy: {}, {} vectors)",
                    "Multi-Query Search Results".bold().underline(),
                    strategy.green(),
                    parsed_vectors.len()
                );
                if results.is_empty() {
                    println!("  No results found.\n");
                } else {
                    for (i, r) in results.iter().enumerate() {
                        println!(
                            "  {}. ID: {} (score: {:.4})",
                            i + 1,
                            r.point.id.to_string().green(),
                            r.score
                        );
                        if let Some(payload) = &r.point.payload {
                            println!("     Payload: {}", payload);
                        }
                    }
                    println!("\n  Total: {} result(s)\n", results.len());
                }
            }
        }
        Commands::CreateMetadataCollection { path, name } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            db.create_collection_typed(&name, &velesdb_core::CollectionType::MetadataOnly)?;

            println!(
                "{} Collection '{}' created (metadata-only)",
                "✅".green(),
                name.cyan()
            );
        }
        Commands::Get {
            path,
            collection,
            id,
            format,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let col = db
                .get_collection(&collection)
                .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;

            let points = col.get(&[id]);

            if format == "json" {
                if let Some(point) = points.into_iter().flatten().next() {
                    let output = serde_json::json!({
                        "id": point.id,
                        "vector": point.vector,
                        "payload": point.payload
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else {
                    println!("null");
                }
            } else {
                if let Some(point) = points.into_iter().flatten().next() {
                    println!("\n{}", "Point Found".bold().underline());
                    println!("  ID: {}", point.id.to_string().green());
                    println!("  Vector: [{} dimensions]", point.vector.len());
                    if let Some(payload) = &point.payload {
                        println!("  Payload: {}", payload);
                    }
                } else {
                    println!("{} Point with ID {} not found", "❌".red(), id);
                }
            }
        }
    }

    Ok(())
}
