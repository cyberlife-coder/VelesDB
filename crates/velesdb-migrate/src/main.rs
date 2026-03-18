//! VelesDB Migration CLI
//!
//! CLI tool for migrating vectors from other databases to VelesDB.
//! Pedantic lints relaxed for CLI ergonomics.

// CLI tool - relax pedantic/nursery lints for ergonomics
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]

mod templates;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use velesdb_migrate::{MigrationConfig, Pipeline, Wizard};

#[derive(Parser)]
#[command(name = "velesdb-migrate")]
#[command(author = "Wiscale France <contact@wiscale.fr>")]
#[command(version)]
#[command(about = "Migrate vectors from other databases to VelesDB", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Dry run mode (don't write to destination)
    #[arg(long)]
    dry_run: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Batch size override
    #[arg(long)]
    batch_size: Option<usize>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive migration wizard (recommended)
    Wizard,

    /// Run migration from config file
    Run {
        /// Configuration file path
        #[arg(short, long, value_name = "FILE")]
        config: PathBuf,
    },

    /// Validate configuration file
    Validate {
        /// Configuration file path
        #[arg(short, long, value_name = "FILE")]
        config: PathBuf,
    },

    /// Show schema from source
    Schema {
        /// Configuration file path
        #[arg(short, long, value_name = "FILE")]
        config: PathBuf,
    },

    /// Generate example configuration
    Init {
        /// Source type (qdrant, pinecone, weaviate, milvus, chromadb, pgvector, supabase)
        #[arg(short, long)]
        source: String,

        /// Output file path
        #[arg(short, long, default_value = "migration.yaml")]
        output: PathBuf,
    },

    /// Auto-detect schema from source and generate config
    Detect {
        /// Source type (supabase, qdrant, chromadb, pinecone, weaviate, milvus)
        #[arg(short, long)]
        source: String,

        /// Source URL
        #[arg(short, long)]
        url: String,

        /// Collection/table/index name
        #[arg(short = 'n', long)]
        collection: String,

        /// API key (if required)
        #[arg(short, long)]
        api_key: Option<String>,

        /// Output config file path
        #[arg(short, long, default_value = "migration.yaml")]
        output: PathBuf,

        /// Destination path for VelesDB
        #[arg(long, default_value = "./velesdb_data")]
        dest_path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let level = if cli.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    match cli.command {
        Some(Commands::Wizard) => run_wizard().await?,
        Some(Commands::Run { config }) => {
            run_migration(&config, cli.dry_run, cli.batch_size).await?;
        }
        Some(Commands::Validate { config }) => validate_config(&config)?,
        Some(Commands::Schema { config }) => show_schema(&config).await?,
        Some(Commands::Init { source, output }) => generate_config(&source, &output)?,
        Some(Commands::Detect {
            source,
            url,
            collection,
            api_key,
            output,
            dest_path,
        }) => {
            auto_detect_and_generate(
                &source,
                &url,
                &collection,
                api_key.as_deref(),
                &output,
                &dest_path,
            )
            .await?;
        }
        None => {
            if let Some(config) = cli.config {
                run_migration(&config, cli.dry_run, cli.batch_size).await?;
            } else {
                eprintln!("Usage: velesdb-migrate --config <FILE> or velesdb-migrate <COMMAND>");
                eprintln!("Try 'velesdb-migrate --help' for more information.");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

async fn run_wizard() -> anyhow::Result<()> {
    let wizard = Wizard::new();
    wizard.run().await.map_err(|e| anyhow::anyhow!("{}", e))
}

async fn run_migration(
    config_path: &PathBuf,
    dry_run: bool,
    batch_size: Option<usize>,
) -> anyhow::Result<()> {
    info!("Loading configuration from {:?}", config_path);

    let mut config = MigrationConfig::from_file(config_path)?;

    if dry_run {
        config.options.dry_run = true;
    }

    if let Some(bs) = batch_size {
        config.options.batch_size = bs;
    }

    config.validate()?;

    info!("Starting migration...");

    let mut pipeline = Pipeline::new(config)?;
    let stats = pipeline.run().await?;

    println!("\n✅ Migration Complete!");
    println!("   Extracted: {}", stats.extracted);
    println!("   Loaded:    {}", stats.loaded);
    println!("   Failed:    {}", stats.failed);
    println!("   Duration:  {:.2}s", stats.duration_secs);
    println!("   Throughput: {:.0} vectors/sec", stats.throughput());

    Ok(())
}

fn validate_config(config_path: &PathBuf) -> anyhow::Result<()> {
    info!("Validating configuration from {:?}", config_path);

    let config = MigrationConfig::from_file(config_path)?;
    config.validate()?;

    println!("✅ Configuration is valid!");
    println!("   Source: {:?}", std::mem::discriminant(&config.source));
    println!("   Destination: {:?}", config.destination.path);
    println!("   Collection: {}", config.destination.collection);
    println!("   Dimension: {}", config.destination.dimension);

    Ok(())
}

async fn show_schema(config_path: &PathBuf) -> anyhow::Result<()> {
    info!("Loading configuration from {:?}", config_path);

    let config = MigrationConfig::from_file(config_path)?;
    let mut connector = velesdb_migrate::connectors::create_connector(&config.source)?;

    connector.connect().await?;
    let schema = connector.get_schema().await?;
    connector.close().await?;

    println!("\n📊 Source Schema:");
    println!("   Type:       {}", schema.source_type);
    println!("   Collection: {}", schema.collection);
    println!("   Dimension:  {}", schema.dimension);
    println!(
        "   Count:      {}",
        schema
            .total_count
            .map_or("unknown".to_string(), |c| c.to_string())
    );

    if !schema.fields.is_empty() {
        println!("   Fields:");
        for field in &schema.fields {
            println!("     - {} ({})", field.name, field.field_type);
        }
    }

    Ok(())
}

fn generate_config(source: &str, output: &PathBuf) -> anyhow::Result<()> {
    let template = match templates::get_template(source) {
        Some(t) => t,
        None => {
            error!("Unknown source type: {}", source);
            eprintln!("Supported sources: qdrant, pinecone, weaviate, milvus, chromadb, pgvector, supabase");
            std::process::exit(1);
        }
    };

    std::fs::write(output, template)?;
    println!("✅ Generated configuration: {:?}", output);
    println!(
        "   Edit the file and run: velesdb-migrate run --config {:?}",
        output
    );

    Ok(())
}

async fn auto_detect_and_generate(
    source_type: &str,
    url: &str,
    collection: &str,
    api_key: Option<&str>,
    output: &std::path::Path,
    dest_path: &std::path::Path,
) -> anyhow::Result<()> {
    use velesdb_migrate::connectors::create_connector;

    println!("🔍 Auto-detecting schema from {} source...", source_type);
    println!("   URL: {}", url);
    println!("   Collection: {}", collection);

    let source_config = build_source_config(source_type, url, collection, api_key)?;

    let mut connector = create_connector(&source_config)?;

    println!("\n🔌 Connecting to source...");
    connector.connect().await?;

    println!("📊 Fetching schema...");
    let schema = connector.get_schema().await?;
    connector.close().await?;

    templates::print_schema_summary(&schema);

    let params = templates::AutoConfigParams {
        source_type,
        url,
        collection,
        api_key,
        dest_path,
        schema: &schema,
    };
    let config_yaml = templates::generate_auto_config(&params);

    std::fs::write(output, &config_yaml)?;

    println!("\n📝 Configuration generated: {:?}", output);
    println!("\n💡 Next steps:");
    println!("   1. Review and edit the config file");
    println!("   2. Verify column names (vector_column, id_column, payload_columns)");
    println!(
        "   3. Run: velesdb-migrate run --config {:?} --dry-run",
        output
    );
    println!("   4. Run: velesdb-migrate run --config {:?}", output);

    Ok(())
}

/// Builds a `SourceConfig` from CLI arguments for the detect command.
fn build_source_config(
    source_type: &str,
    url: &str,
    collection: &str,
    api_key: Option<&str>,
) -> anyhow::Result<velesdb_migrate::config::SourceConfig> {
    use velesdb_migrate::config::*;

    match source_type.to_lowercase().as_str() {
        "supabase" => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!("Supabase requires --api-key"))?;
            Ok(SourceConfig::Supabase(SupabaseConfig {
                url: url.to_string(),
                api_key: key.to_string(),
                table: collection.to_string(),
                vector_column: "embedding".to_string(),
                id_column: "id".to_string(),
                payload_columns: vec![],
            }))
        }
        "qdrant" => Ok(SourceConfig::Qdrant(QdrantConfig {
            url: url.to_string(),
            collection: collection.to_string(),
            api_key: api_key.map(String::from),
            payload_fields: vec![],
        })),
        "chromadb" => Ok(SourceConfig::ChromaDB(ChromaDBConfig {
            url: url.to_string(),
            collection: collection.to_string(),
            tenant: None,
            database: None,
        })),
        "pinecone" => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!("Pinecone requires --api-key"))?;
            Ok(SourceConfig::Pinecone(PineconeConfig {
                api_key: key.to_string(),
                environment: String::new(),
                index: collection.to_string(),
                namespace: None,
                base_url: None,
            }))
        }
        "weaviate" => Ok(SourceConfig::Weaviate(WeaviateConfig {
            url: url.to_string(),
            class_name: collection.to_string(),
            api_key: api_key.map(String::from),
            properties: vec![],
        })),
        "milvus" => Ok(SourceConfig::Milvus(MilvusConfig {
            url: url.to_string(),
            collection: collection.to_string(),
            username: None,
            password: None,
        })),
        _ => {
            eprintln!("❌ Unknown source type: {}", source_type);
            eprintln!("   Supported: supabase, qdrant, chromadb, pinecone, weaviate, milvus");
            std::process::exit(1);
        }
    }
}
