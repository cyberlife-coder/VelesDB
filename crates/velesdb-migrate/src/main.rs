//! VelesDB Migration CLI

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use velesdb_migrate::{MigrationConfig, Pipeline};

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
        Some(Commands::Run { config }) => {
            run_migration(&config, cli.dry_run, cli.batch_size).await?;
        }
        Some(Commands::Validate { config }) => {
            validate_config(&config)?;
        }
        Some(Commands::Schema { config }) => {
            show_schema(&config).await?;
        }
        Some(Commands::Init { source, output }) => {
            generate_config(&source, &output)?;
        }
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
            // Default: run migration if config provided
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
    let template = match source.to_lowercase().as_str() {
        "qdrant" => QDRANT_TEMPLATE,
        "pinecone" => PINECONE_TEMPLATE,
        "weaviate" => WEAVIATE_TEMPLATE,
        "milvus" => MILVUS_TEMPLATE,
        "chromadb" => CHROMADB_TEMPLATE,
        "pgvector" => PGVECTOR_TEMPLATE,
        "supabase" => SUPABASE_TEMPLATE,
        _ => {
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
    output: &PathBuf,
    dest_path: &PathBuf,
) -> anyhow::Result<()> {
    use velesdb_migrate::config::*;
    use velesdb_migrate::connectors::create_connector;

    println!("🔍 Auto-detecting schema from {} source...", source_type);
    println!("   URL: {}", url);
    println!("   Collection: {}", collection);

    // Create source config based on type
    let source_config = match source_type.to_lowercase().as_str() {
        "supabase" => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!("Supabase requires --api-key"))?;
            SourceConfig::Supabase(SupabaseConfig {
                url: url.to_string(),
                api_key: key.to_string(),
                table: collection.to_string(),
                vector_column: "embedding".to_string(),
                id_column: "id".to_string(),
                payload_columns: vec![],
            })
        }
        "qdrant" => SourceConfig::Qdrant(QdrantConfig {
            url: url.to_string(),
            collection: collection.to_string(),
            api_key: api_key.map(String::from),
            payload_fields: vec![],
        }),
        "chromadb" => SourceConfig::ChromaDB(ChromaDBConfig {
            url: url.to_string(),
            collection: collection.to_string(),
            tenant: None,
            database: None,
        }),
        "pinecone" => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!("Pinecone requires --api-key"))?;
            SourceConfig::Pinecone(PineconeConfig {
                api_key: key.to_string(),
                environment: "".to_string(),
                index: collection.to_string(),
                namespace: None,
            })
        }
        "weaviate" => SourceConfig::Weaviate(WeaviateConfig {
            url: url.to_string(),
            class_name: collection.to_string(),
            api_key: api_key.map(String::from),
            properties: vec![],
        }),
        "milvus" => SourceConfig::Milvus(MilvusConfig {
            url: url.to_string(),
            collection: collection.to_string(),
            username: None,
            password: None,
        }),
        _ => {
            eprintln!("❌ Unknown source type: {}", source_type);
            eprintln!("   Supported: supabase, qdrant, chromadb, pinecone, weaviate, milvus");
            std::process::exit(1);
        }
    };

    // Connect and detect schema
    let mut connector = create_connector(&source_config)?;

    println!("\n🔌 Connecting to source...");
    connector.connect().await?;

    println!("📊 Fetching schema...");
    let schema = connector.get_schema().await?;

    connector.close().await?;

    // Display detected schema
    println!("\n✅ Schema Detected!");
    println!("┌─────────────────────────────────────────────");
    println!("│ Source Type:  {}", schema.source_type);
    println!("│ Collection:   {}", schema.collection);
    println!(
        "│ Dimension:    {}",
        if schema.dimension > 0 {
            schema.dimension.to_string()
        } else {
            "auto-detect on first batch".to_string()
        }
    );
    println!(
        "│ Total Count:  {}",
        schema
            .total_count
            .map_or("unknown".to_string(), |c| format!("{} vectors", c))
    );
    println!("├─────────────────────────────────────────────");

    if !schema.fields.is_empty() {
        println!("│ Detected Metadata Fields:");
        for field in &schema.fields {
            let indexed = if field.indexed { " [indexed]" } else { "" };
            println!("│   • {} ({}){}", field.name, field.field_type, indexed);
        }
    } else {
        println!("│ Metadata Fields: (all fields will be migrated)");
    }
    println!("└─────────────────────────────────────────────");

    // Generate config YAML
    let dimension = if schema.dimension > 0 {
        schema.dimension
    } else {
        768
    };

    // Use detected columns from schema, fallback to heuristics
    let detected_vector_col = schema.vector_column.clone().unwrap_or_else(|| {
        schema
            .fields
            .iter()
            .find(|f| {
                let lower = f.name.to_lowercase();
                lower.contains("vector") || lower.contains("embedding") || lower.contains("emb")
            })
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "embedding".to_string())
    });

    let detected_id_col = schema.id_column.clone().unwrap_or_else(|| {
        schema
            .fields
            .iter()
            .find(|f| {
                f.name.to_lowercase().contains("id")
                    || f.name.to_lowercase() == "code"
                    || f.name.to_lowercase().ends_with("_id")
            })
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "id".to_string())
    });

    // Filter out ID and vector columns from payload
    let payload_fields: Vec<_> = schema
        .fields
        .iter()
        .filter(|f| f.name != detected_id_col && f.name != detected_vector_col)
        .collect();

    let fields_list = if payload_fields.is_empty() {
        "    # All metadata fields will be migrated automatically".to_string()
    } else {
        payload_fields
            .iter()
            .map(|f| format!("    - {}", f.name))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let config_yaml = match source_type.to_lowercase().as_str() {
        "supabase" => format!(
            r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: Supabase
# Detected: {} vectors, {}D

source:
  type: supabase
  url: {}
  api_key: ${{SUPABASE_SERVICE_KEY}}  # Set env var for security
  table: {}
  vector_column: {}
  id_column: {}
  payload_columns:
{}

destination:
  path: {}
  collection: {}
  dimension: {}
  metric: cosine
  storage_mode: full

options:
  batch_size: 500
  workers: 2
  continue_on_error: false
"#,
            schema
                .total_count
                .map_or("?".to_string(), |c| c.to_string()),
            dimension,
            url,
            collection,
            detected_vector_col,
            detected_id_col,
            fields_list,
            dest_path.display(),
            collection,
            dimension
        ),
        "qdrant" => format!(
            r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: Qdrant
# Detected: {} vectors, {}D

source:
  type: qdrant
  url: {}
  collection: {}
{}
  payload_fields: []  # Empty = all fields

destination:
  path: {}
  collection: {}
  dimension: {}
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
  workers: 4
"#,
            schema
                .total_count
                .map_or("?".to_string(), |c| c.to_string()),
            dimension,
            url,
            collection,
            api_key.map_or("  # api_key: your-key".to_string(), |k| format!(
                "  api_key: {}",
                k
            )),
            dest_path.display(),
            collection,
            dimension
        ),
        "chromadb" => format!(
            r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: ChromaDB
# Detected: {} vectors, {}D

source:
  type: chromadb
  url: {}
  collection: {}

destination:
  path: {}
  collection: {}
  dimension: {}
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
"#,
            schema
                .total_count
                .map_or("?".to_string(), |c| c.to_string()),
            dimension,
            url,
            collection,
            dest_path.display(),
            collection,
            dimension
        ),
        "weaviate" => format!(
            r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: Weaviate
# Detected: {} objects, {}D

source:
  type: weaviate
  url: {}
  class_name: {}
{}
  properties:  # Detected properties:
{}

destination:
  path: {}
  collection: {}
  dimension: {}
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
"#,
            schema
                .total_count
                .map_or("?".to_string(), |c| c.to_string()),
            dimension,
            url,
            collection,
            api_key.map_or("  # api_key: your-key".to_string(), |k| format!(
                "  api_key: {}",
                k
            )),
            fields_list,
            dest_path.display(),
            collection,
            dimension
        ),
        _ => format!(
            r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: {}
# Detected: {} vectors, {}D

source:
  type: {}
  url: {}
  collection: {}

destination:
  path: {}
  collection: {}
  dimension: {}
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
"#,
            source_type,
            schema
                .total_count
                .map_or("?".to_string(), |c| c.to_string()),
            dimension,
            source_type.to_lowercase(),
            url,
            collection,
            dest_path.display(),
            collection,
            dimension
        ),
    };

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

const QDRANT_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Qdrant Source
source:
  type: qdrant
  url: http://localhost:6333
  collection: your_collection
  # api_key: your-api-key  # Optional

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine  # cosine, euclidean, or dot
  storage_mode: full  # full, sq8, or binary

options:
  batch_size: 1000
  workers: 4
  dry_run: false
  continue_on_error: false
"#;

const PINECONE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Pinecone Source
source:
  type: pinecone
  api_key: your-pinecone-api-key
  environment: us-east-1-aws
  index: your-index-name
  # namespace: optional-namespace

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 100  # Pinecone has lower batch limits
  workers: 2
"#;

const WEAVIATE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Weaviate Source
source:
  type: weaviate
  url: http://localhost:8080
  class_name: Document
  # api_key: your-api-key  # Optional
  properties:
    - title
    - content

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const MILVUS_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Milvus Source
source:
  type: milvus
  url: http://localhost:19530
  collection: your_collection
  # username: root
  # password: milvus

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const CHROMADB_TEMPLATE: &str = r#"# VelesDB Migration Configuration - ChromaDB Source
source:
  type: chromadb
  url: http://localhost:8000
  collection: your_collection
  # tenant: default_tenant
  # database: default_database

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const PGVECTOR_TEMPLATE: &str = r#"# VelesDB Migration Configuration - pgvector Source
# Requires: velesdb-migrate --features postgres
source:
  type: pgvector
  connection_string: postgres://user:password@localhost:5432/database
  table: embeddings
  vector_column: embedding
  id_column: id
  payload_columns:
    - title
    - content
  # filter: "created_at > '2024-01-01'"

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const SUPABASE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Supabase Source
source:
  type: supabase
  url: https://your-project.supabase.co
  api_key: your-service-role-key
  table: documents
  vector_column: embedding
  id_column: id
  payload_columns:
    - title
    - content

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;
