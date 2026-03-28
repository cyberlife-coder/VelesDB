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

use clap::{CommandFactory, Parser};
use clap_complete::generate;
use std::io;
use std::path::PathBuf;
use velesdb_core::{DistanceMetric, StorageMode};

use commands::{Commands, IndexAction, LicenseAction, SimdAction};

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

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Reason: CLI entry point with command dispatch
fn main() -> anyhow::Result<()> {
    // Non-blocking update check (background thread, 2s timeout).
    // Disable: VELESDB_NO_UPDATE_CHECK=1 or [update_check] enabled=false in config.
    #[cfg(feature = "update-check")]
    velesdb_core::spawn_update_check(
        velesdb_core::UpdateCheckConfig::default(),
        std::path::PathBuf::from("."),
        "core".to_string(),
    );

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
                let type_label = collection_helpers::collection_type_label(&db, &name);
                match collection_helpers::resolve_collection(&db, &name) {
                    Some(collection_helpers::TypedCollection::Vector(col)) => {
                        let config = col.config();
                        println!(
                            "  - {} [{}] ({} dims, {:?}, {} points)",
                            config.name,
                            type_label,
                            config.dimension,
                            config.metric,
                            config.point_count
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
        }
        Commands::List { path, format } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let collections = db.list_collections();

            if format == "json" {
                let data: Vec<_> = collections
                    .iter()
                    .map(|name| {
                        let type_label = collection_helpers::collection_type_label(&db, name);
                        match collection_helpers::resolve_collection(&db, name) {
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
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                println!("\n{}", "Collections".bold().underline());
                if collections.is_empty() {
                    println!("  No collections found.\n");
                } else {
                    for name in &collections {
                        let type_label = collection_helpers::collection_type_label(&db, name);
                        match collection_helpers::resolve_collection(&db, name) {
                            Some(collection_helpers::TypedCollection::Vector(col)) => {
                                let cfg = col.config();
                                println!(
                                    "  {} {} [{}] ({} dims, {:?}, {} points)",
                                    "•".cyan(),
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
                                    "•".cyan(),
                                    col.name().green(),
                                    type_label.cyan(),
                                    col.get_edges(None).len()
                                );
                            }
                            Some(collection_helpers::TypedCollection::Metadata(col)) => {
                                println!(
                                    "  {} {} [{}] ({} items)",
                                    "•".cyan(),
                                    col.name().green(),
                                    type_label.cyan(),
                                    col.len()
                                );
                            }
                            None => {
                                println!(
                                    "  {} {} [{}]",
                                    "•".cyan(),
                                    name.green(),
                                    type_label.cyan()
                                );
                            }
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
            let typed = collection_helpers::resolve_collection(&db, &collection)
                .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;
            let type_label = collection_helpers::collection_type_label(&db, &collection);

            match typed {
                collection_helpers::TypedCollection::Vector(col) => {
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
                        println!("\n{}", "Collection Details".bold().underline());
                        println!("  {} {}", "Name:".cyan(), cfg.name.green());
                        println!("  {} {}", "Type:".cyan(), type_label.green());
                        println!("  {} {}", "Dimension:".cyan(), cfg.dimension);
                        println!("  {} {:?}", "Metric:".cyan(), cfg.metric);
                        println!("  {} {}", "Point Count:".cyan(), cfg.point_count);
                        println!("  {} {:?}", "Storage Mode:".cyan(), cfg.storage_mode);

                        let estimated_mb =
                            (cfg.point_count * cfg.dimension * 4) as f64 / 1_000_000.0;
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
                collection_helpers::TypedCollection::Graph(col) => {
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
                }
                collection_helpers::TypedCollection::Metadata(col) => {
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
                }
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
            let col = db.get_vector_collection(&collection).ok_or_else(|| {
                anyhow::anyhow!(
                    "Vector collection '{}' not found. Export requires a vector collection.",
                    collection
                )
            })?;

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
                .get_vector_collection(&collection)
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
            let query_refs: Vec<&[f32]> = parsed_vectors.iter().map(Vec::as_slice).collect();

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
            db.create_metadata_collection(&name)?;

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
                .get_vector_collection(&collection)
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
            } else if let Some(point) = points.into_iter().flatten().next() {
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
        Commands::Graph { action } => {
            graph::handle(action)?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "velesdb", &mut io::stdout());
        }
        Commands::Simd { action } => {
            use colored::Colorize;

            match action {
                SimdAction::Info => {
                    println!("\n{}", "SIMD Native Configuration".bold().underline());
                    println!("  Using simd_native with tiered dispatch:");
                    println!("  - AVX-512: 4/2/1 accumulators based on vector size");
                    println!("  - AVX2: 4-acc (>1024D), 2-acc (64-1023D), 1-acc (<64D)");
                    println!("  - ARM NEON: 128-bit SIMD");
                    println!("  - Scalar: fallback for small vectors");
                    println!("\n{}", "Available Functions:".cyan());
                    println!("  - dot_product_native()");
                    println!("  - cosine_similarity_native()");
                    println!("  - euclidean_native()");
                    println!("  - hamming_distance_native()");
                    println!("  - jaccard_similarity_native()");
                    println!("  - batch_dot_product_native() (with prefetching)");
                    println!();
                }
                SimdAction::Benchmark => {
                    println!("{}", "SIMD micro-benchmarks removed.".yellow());
                    println!("Use 'cargo bench --bench simd_benchmark' for detailed benchmarks.");
                    println!();
                }
            }
        }
        Commands::CreateVectorCollection {
            path,
            name,
            dimension,
            metric,
            storage,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            db.create_vector_collection_with_options(
                &name,
                dimension,
                metric.into(),
                storage.into(),
            )?;

            println!(
                "{} Vector collection '{}' created ({} dims, {:?}, {:?})",
                "✅".green(),
                name.cyan(),
                dimension,
                DistanceMetric::from(metric),
                StorageMode::from(storage),
            );
        }
        Commands::CreateGraphCollection {
            path,
            name,
            schemaless,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let schema = if schemaless {
                velesdb_core::GraphSchema::schemaless()
            } else {
                // Reason: Strict schema from file is planned for future; default to schemaless.
                velesdb_core::GraphSchema::schemaless()
            };
            db.create_graph_collection(&name, schema)?;

            println!(
                "{} Graph collection '{}' created (schemaless: {})",
                "✅".green(),
                name.cyan(),
                schemaless,
            );
        }
        Commands::DeleteCollection { path, name, force } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;

            if !force {
                println!(
                    "Are you sure you want to delete collection '{}'? This cannot be undone.",
                    name.yellow()
                );
                print!("Type 'yes' to confirm: ");
                use std::io::Write;
                std::io::stdout().flush()?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if input.trim() != "yes" {
                    println!("{} Deletion cancelled.", "ℹ️".cyan());
                    return Ok(());
                }
            }

            db.delete_collection(&name)?;
            println!("{} Collection '{}' deleted.", "✅".green(), name.cyan());
        }
        Commands::Explain {
            path,
            query,
            format,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let parsed = velesdb_core::velesql::Parser::parse(&query)
                .map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;

            let plan = db
                .explain_query(&parsed)
                .map_err(|e| anyhow::anyhow!("Explain error: {}", e))?;

            if format == "json" {
                let json = plan
                    .to_json()
                    .map_err(|e| anyhow::anyhow!("JSON serialization error: {}", e))?;
                println!("{}", json);
            } else {
                println!("\n{}", "Query Execution Plan".bold().underline());
                println!("{}", plan.to_tree());
                println!(
                    "  {} {:.3} ms",
                    "Estimated cost:".cyan(),
                    plan.estimated_cost_ms
                );
                if let Some(idx) = &plan.index_used {
                    println!("  {} {:?}", "Index used:".cyan(), idx);
                }
                println!("  {} {:?}", "Filter strategy:".cyan(), plan.filter_strategy);
                if let Some(hit) = plan.cache_hit {
                    println!(
                        "  {} {}",
                        "Cache hit:".cyan(),
                        if hit { "yes".green() } else { "no".yellow() }
                    );
                }
                if let Some(reuse) = plan.plan_reuse_count {
                    println!("  {} {}", "Plan reuse count:".cyan(), reuse);
                }
                println!();
            }
        }
        Commands::Analyze {
            path,
            collection,
            format,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let stats = db
                .analyze_collection(&collection)
                .map_err(|e| anyhow::anyhow!("Analyze error: {}", e))?;

            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
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

                if !stats.index_stats.is_empty() {
                    println!("\n  {}", "Indexes:".cyan().bold());
                    for (name, idx) in &stats.index_stats {
                        println!(
                            "    {} ({}) — {} entries, depth {}, {} bytes",
                            name.green(),
                            idx.index_type,
                            idx.entry_count,
                            idx.depth,
                            idx.size_bytes
                        );
                    }
                }

                if !stats.column_stats.is_empty() {
                    println!("\n  {}", "Columns:".cyan().bold());
                    for (name, col) in &stats.column_stats {
                        println!(
                            "    {} — {} distinct, {} nulls",
                            name.green(),
                            col.distinct_count,
                            col.null_count
                        );
                    }
                }

                if let Some(ts) = stats.last_analyzed_epoch_ms {
                    println!("\n  {} epoch {} ms", "Last analyzed:".cyan(), ts);
                }
                println!();
            }
        }
        Commands::DeletePoints {
            path,
            collection,
            ids,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let col = db
                .get_vector_collection(&collection)
                .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

            col.delete(&ids)
                .map_err(|e| anyhow::anyhow!("Delete failed: {}", e))?;

            println!(
                "{} Deleted {} point(s) from '{}'",
                "✅".green(),
                ids.len(),
                collection.cyan()
            );
        }
        Commands::Upsert {
            path,
            collection,
            id,
            vector,
            payload,
        } => {
            use colored::Colorize;

            let db = velesdb_core::Database::open(&path)?;
            let col = db
                .get_vector_collection(&collection)
                .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

            let vec_data: Vec<f32> = match vector {
                Some(v) => serde_json::from_str(&v)
                    .map_err(|e| anyhow::anyhow!("Invalid vector JSON: {}", e))?,
                None => vec![],
            };

            let payload_data: Option<serde_json::Value> = match payload {
                Some(p) => Some(
                    serde_json::from_str(&p)
                        .map_err(|e| anyhow::anyhow!("Invalid payload JSON: {}", e))?,
                ),
                None => None,
            };

            let point = velesdb_core::Point::new(id, vec_data, payload_data);
            col.upsert(vec![point])
                .map_err(|e| anyhow::anyhow!("Upsert failed: {}", e))?;

            println!(
                "{} Upserted point {} into '{}'",
                "✅".green(),
                id.to_string().green(),
                collection.cyan()
            );
        }
        Commands::Index { action } => {
            use colored::Colorize;

            match action {
                IndexAction::Create {
                    path,
                    collection,
                    field,
                    index_type,
                    label,
                } => {
                    let db = velesdb_core::Database::open(&path)?;
                    let col = db.get_vector_collection(&collection).ok_or_else(|| {
                        anyhow::anyhow!("Vector collection '{}' not found", collection)
                    })?;

                    match index_type {
                        cli_types::IndexTypeArg::Secondary => {
                            col.create_index(&field)
                                .map_err(|e| anyhow::anyhow!("Create index failed: {}", e))?;
                            println!(
                                "{} Secondary index created on field '{}' in '{}'",
                                "✅".green(),
                                field.cyan(),
                                collection.cyan()
                            );
                        }
                        cli_types::IndexTypeArg::Property => {
                            let lbl = label.as_deref().unwrap_or("default");
                            col.create_property_index(lbl, &field)
                                .map_err(|e| anyhow::anyhow!("Create index failed: {}", e))?;
                            println!(
                                "{} Property index created on '{}:{}' in '{}'",
                                "✅".green(),
                                lbl.cyan(),
                                field.cyan(),
                                collection.cyan()
                            );
                        }
                        cli_types::IndexTypeArg::Range => {
                            let lbl = label.as_deref().unwrap_or("default");
                            col.create_range_index(lbl, &field)
                                .map_err(|e| anyhow::anyhow!("Create index failed: {}", e))?;
                            println!(
                                "{} Range index created on '{}:{}' in '{}'",
                                "✅".green(),
                                lbl.cyan(),
                                field.cyan(),
                                collection.cyan()
                            );
                        }
                    }
                }
                IndexAction::Drop {
                    path,
                    collection,
                    label,
                    property,
                } => {
                    let db = velesdb_core::Database::open(&path)?;
                    let col = db.get_vector_collection(&collection).ok_or_else(|| {
                        anyhow::anyhow!("Vector collection '{}' not found", collection)
                    })?;

                    let dropped = col
                        .drop_index(&label, &property)
                        .map_err(|e| anyhow::anyhow!("Drop index failed: {}", e))?;

                    if dropped {
                        println!(
                            "{} Index '{}:{}' dropped from '{}'",
                            "✅".green(),
                            label.cyan(),
                            property.cyan(),
                            collection.cyan()
                        );
                    } else {
                        println!(
                            "{} No index '{}:{}' found in '{}'",
                            "⚠️".yellow(),
                            label,
                            property,
                            collection
                        );
                    }
                }
                IndexAction::List {
                    path,
                    collection,
                    format,
                } => {
                    let db = velesdb_core::Database::open(&path)?;
                    let col = db.get_vector_collection(&collection).ok_or_else(|| {
                        anyhow::anyhow!("Vector collection '{}' not found", collection)
                    })?;

                    let indexes = col.list_indexes();

                    if format == "json" {
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
                    } else {
                        println!("\n{}", "Indexes".bold().underline());
                        if indexes.is_empty() {
                            println!("  No indexes found.\n");
                        } else {
                            for idx in &indexes {
                                println!(
                                    "  {} {}:{} ({}) — {} unique values, {} bytes",
                                    "•".cyan(),
                                    idx.label.green(),
                                    idx.property.green(),
                                    idx.index_type,
                                    idx.cardinality,
                                    idx.memory_bytes
                                );
                            }
                            println!("\n  Total: {} index(es)\n", indexes.len());
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cli_types::{MetricArg, StorageModeArg};

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
