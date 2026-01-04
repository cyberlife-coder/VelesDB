//! Interactive migration wizard for zero-config migrations.
//!
//! The wizard guides users through the migration process step by step,
//! auto-detecting schema and configuration options.

mod discovery;
mod prompts;
mod ui;

pub use discovery::SourceDiscovery;
pub use prompts::WizardPrompts;
pub use ui::WizardUI;

use crate::config::{
    DestinationConfig, DistanceMetric, MigrationOptions, SourceConfig, StorageMode,
};
use crate::connectors::{create_connector, SourceSchema};
use crate::error::Result;
use crate::pipeline::Pipeline;
use crate::MigrationConfig;
use std::path::PathBuf;

/// Supported source types for the wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// Supabase (PostgreSQL + pgvector via PostgREST).
    Supabase,
    /// Qdrant vector database.
    Qdrant,
    /// Pinecone serverless/pod indexes.
    Pinecone,
    /// Weaviate vector database.
    Weaviate,
    /// Milvus / Zilliz Cloud.
    Milvus,
    /// ChromaDB vector database.
    ChromaDB,
    /// PostgreSQL with pgvector extension (direct SQL).
    PgVector,
    /// JSON file import.
    JsonFile,
    /// CSV file import.
    CsvFile,
    /// MongoDB Atlas Vector Search.
    MongoDB,
    /// Elasticsearch/OpenSearch with vector search.
    Elasticsearch,
    /// Redis Vector Search (Redis Stack).
    Redis,
}

impl SourceType {
    /// Returns all available source types.
    pub fn all() -> Vec<Self> {
        vec![
            Self::Supabase,
            Self::Qdrant,
            Self::Pinecone,
            Self::Weaviate,
            Self::Milvus,
            Self::ChromaDB,
            Self::PgVector,
            Self::JsonFile,
            Self::CsvFile,
            Self::MongoDB,
            Self::Elasticsearch,
            Self::Redis,
        ]
    }

    /// Display name for the source type.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Supabase => "Supabase (PostgreSQL + pgvector)",
            Self::Qdrant => "Qdrant",
            Self::Pinecone => "Pinecone",
            Self::Weaviate => "Weaviate",
            Self::Milvus => "Milvus / Zilliz Cloud",
            Self::ChromaDB => "ChromaDB",
            Self::PgVector => "PostgreSQL (pgvector direct)",
            Self::JsonFile => "JSON File (local import)",
            Self::CsvFile => "CSV File (local import)",
            Self::MongoDB => "MongoDB Atlas Vector Search",
            Self::Elasticsearch => "Elasticsearch / OpenSearch",
            Self::Redis => "Redis Vector Search",
        }
    }

    /// Short name for CLI.
    pub fn short_name(&self) -> &'static str {
        match self {
            Self::Supabase => "supabase",
            Self::Qdrant => "qdrant",
            Self::Pinecone => "pinecone",
            Self::Weaviate => "weaviate",
            Self::Milvus => "milvus",
            Self::ChromaDB => "chromadb",
            Self::PgVector => "pgvector",
            Self::JsonFile => "json_file",
            Self::CsvFile => "csv_file",
            Self::MongoDB => "mongodb",
            Self::Elasticsearch => "elasticsearch",
            Self::Redis => "redis",
        }
    }

    /// Whether this source requires an API key.
    pub fn requires_api_key(&self) -> bool {
        matches!(self, Self::Supabase | Self::Pinecone | Self::MongoDB)
    }

    /// Whether API key is optional.
    pub fn optional_api_key(&self) -> bool {
        matches!(
            self,
            Self::Qdrant | Self::Weaviate | Self::Milvus | Self::Elasticsearch | Self::Redis
        )
    }
}

/// Configuration collected during wizard interaction.
#[derive(Debug, Clone)]
pub struct WizardConfig {
    /// Selected source type.
    pub source_type: SourceType,
    /// Source URL or connection string.
    pub url: String,
    /// API key (if required by source).
    pub api_key: Option<String>,
    /// Collection/table/index name.
    pub collection: String,
    /// Destination path for VelesDB data.
    pub dest_path: String,
    /// Use SQ8 compression (4x smaller).
    pub use_sq8: bool,
}

/// Interactive migration wizard.
pub struct Wizard {
    ui: WizardUI,
    prompts: WizardPrompts,
}

impl Default for Wizard {
    fn default() -> Self {
        Self::new()
    }
}

impl Wizard {
    /// Creates a new wizard instance.
    pub fn new() -> Self {
        Self {
            ui: WizardUI::new(),
            prompts: WizardPrompts::new(),
        }
    }

    /// Runs the interactive wizard.
    pub async fn run(&self) -> Result<()> {
        self.ui.print_header();

        // Step 1: Select source type
        let source_type = self.prompts.select_source()?;

        // Step 2: Get connection details
        let config = self.prompts.get_connection_details(source_type)?;

        // Step 3: Connect and discover schema
        self.ui.print_connecting(&config.url);
        let source_config = self.build_source_config(&config)?;
        let mut connector = create_connector(&source_config)?;

        connector.connect().await?;
        let schema = connector.get_schema().await?;
        connector.close().await?;

        // Step 4: Show discovered schema
        self.ui.print_schema_discovered(&schema);

        // Step 5: Confirm migration
        if !self.prompts.confirm_migration(&schema, &config)? {
            self.ui.print_cancelled();
            return Ok(());
        }

        // Step 6: Run migration
        self.ui.print_starting_migration();

        let migration_config = self.build_migration_config(&config, &schema)?;
        let mut pipeline = Pipeline::new(migration_config)?;
        let stats = pipeline.run().await?;

        // Step 7: Show results
        self.ui.print_success(&stats, &config);

        Ok(())
    }

    /// Builds source config from wizard config.
    fn build_source_config(&self, config: &WizardConfig) -> Result<SourceConfig> {
        use crate::config::*;

        let source = match config.source_type {
            SourceType::Supabase => SourceConfig::Supabase(SupabaseConfig {
                url: config.url.clone(),
                api_key: config.api_key.clone().unwrap_or_default(),
                table: config.collection.clone(),
                vector_column: "embedding".to_string(),
                id_column: "id".to_string(),
                payload_columns: vec![],
            }),
            SourceType::Qdrant => SourceConfig::Qdrant(QdrantConfig {
                url: config.url.clone(),
                collection: config.collection.clone(),
                api_key: config.api_key.clone(),
                payload_fields: vec![],
            }),
            SourceType::Pinecone => SourceConfig::Pinecone(PineconeConfig {
                api_key: config.api_key.clone().unwrap_or_default(),
                environment: String::new(),
                index: config.collection.clone(),
                namespace: None,
            }),
            SourceType::Weaviate => SourceConfig::Weaviate(WeaviateConfig {
                url: config.url.clone(),
                class_name: config.collection.clone(),
                api_key: config.api_key.clone(),
                properties: vec![],
            }),
            SourceType::Milvus => SourceConfig::Milvus(MilvusConfig {
                url: config.url.clone(),
                collection: config.collection.clone(),
                username: None,
                password: None,
            }),
            SourceType::ChromaDB => SourceConfig::ChromaDB(ChromaDBConfig {
                url: config.url.clone(),
                collection: config.collection.clone(),
                tenant: None,
                database: None,
            }),
            SourceType::PgVector => {
                #[cfg(feature = "postgres")]
                {
                    SourceConfig::PgVector(PgVectorConfig {
                        connection_string: config.url.clone(),
                        table: config.collection.clone(),
                        vector_column: "embedding".to_string(),
                        id_column: "id".to_string(),
                        payload_columns: vec![],
                        filter: None,
                    })
                }
                #[cfg(not(feature = "postgres"))]
                {
                    return Err(crate::error::Error::Config(
                        "pgvector requires --features postgres".to_string(),
                    ));
                }
            }
            SourceType::JsonFile => {
                SourceConfig::JsonFile(crate::connectors::json_file::JsonFileConfig {
                    path: std::path::PathBuf::from(&config.url),
                    array_path: String::new(),
                    id_field: "id".to_string(),
                    vector_field: "vector".to_string(),
                    payload_fields: vec![],
                })
            }
            SourceType::CsvFile => {
                SourceConfig::CsvFile(crate::connectors::csv_file::CsvFileConfig {
                    path: std::path::PathBuf::from(&config.url),
                    id_column: "id".to_string(),
                    vector_column: "vector".to_string(),
                    vector_spread: false,
                    dim_prefix: "dim_".to_string(),
                    delimiter: ',',
                    has_header: true,
                })
            }
            SourceType::MongoDB => {
                SourceConfig::MongoDB(crate::connectors::mongodb::MongoDBConfig {
                    data_api_url: config.url.clone(),
                    api_key: config.api_key.clone().unwrap_or_default(),
                    database: "vectors".to_string(),
                    collection: config.collection.clone(),
                    vector_field: "embedding".to_string(),
                    id_field: "_id".to_string(),
                    payload_fields: vec![],
                    filter: None,
                })
            }
            SourceType::Elasticsearch => {
                SourceConfig::Elasticsearch(crate::connectors::elasticsearch::ElasticsearchConfig {
                    url: config.url.clone(),
                    index: config.collection.clone(),
                    vector_field: "embedding".to_string(),
                    id_field: "_id".to_string(),
                    payload_fields: vec![],
                    username: None,
                    password: None,
                    api_key: config.api_key.clone(),
                    query: None,
                })
            }
            SourceType::Redis => SourceConfig::Redis(crate::connectors::redis::RedisConfig {
                url: config.url.clone(),
                password: config.api_key.clone(),
                index: config.collection.clone(),
                vector_field: "embedding".to_string(),
                key_prefix: "doc:".to_string(),
                payload_fields: vec![],
                filter: None,
            }),
        };

        Ok(source)
    }

    /// Builds full migration config.
    fn build_migration_config(
        &self,
        config: &WizardConfig,
        schema: &SourceSchema,
    ) -> Result<MigrationConfig> {
        let source = self.build_source_config(config)?;

        let storage_mode = if config.use_sq8 {
            StorageMode::SQ8
        } else {
            StorageMode::Full
        };

        let destination = DestinationConfig {
            path: PathBuf::from(&config.dest_path),
            collection: config.collection.clone(),
            dimension: schema.dimension,
            metric: DistanceMetric::Cosine,
            storage_mode,
        };

        let options = MigrationOptions {
            batch_size: 1000,
            workers: 4,
            dry_run: false,
            continue_on_error: false,
            checkpoint_enabled: true,
            checkpoint_path: None,
            field_mappings: std::collections::HashMap::new(),
        };

        Ok(MigrationConfig {
            source,
            destination,
            options,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== SourceType Tests ====================

    #[test]
    fn test_source_type_all_returns_all_variants() {
        // Arrange & Act
        let all = SourceType::all();

        // Assert
        assert_eq!(all.len(), 12);
        assert!(all.contains(&SourceType::Supabase));
        assert!(all.contains(&SourceType::Qdrant));
        assert!(all.contains(&SourceType::Pinecone));
        assert!(all.contains(&SourceType::Weaviate));
        assert!(all.contains(&SourceType::Milvus));
        assert!(all.contains(&SourceType::ChromaDB));
        assert!(all.contains(&SourceType::PgVector));
        assert!(all.contains(&SourceType::JsonFile));
        assert!(all.contains(&SourceType::CsvFile));
        assert!(all.contains(&SourceType::MongoDB));
        assert!(all.contains(&SourceType::Elasticsearch));
        assert!(all.contains(&SourceType::Redis));
    }

    #[test]
    fn test_source_type_display_names() {
        // Arrange & Act & Assert
        assert_eq!(
            SourceType::Supabase.display_name(),
            "Supabase (PostgreSQL + pgvector)"
        );
        assert_eq!(SourceType::Qdrant.display_name(), "Qdrant");
        assert_eq!(SourceType::Pinecone.display_name(), "Pinecone");
        assert_eq!(SourceType::Weaviate.display_name(), "Weaviate");
        assert_eq!(SourceType::Milvus.display_name(), "Milvus / Zilliz Cloud");
        assert_eq!(SourceType::ChromaDB.display_name(), "ChromaDB");
        assert_eq!(
            SourceType::PgVector.display_name(),
            "PostgreSQL (pgvector direct)"
        );
    }

    #[test]
    fn test_source_type_short_names() {
        // Arrange & Act & Assert
        assert_eq!(SourceType::Supabase.short_name(), "supabase");
        assert_eq!(SourceType::Qdrant.short_name(), "qdrant");
        assert_eq!(SourceType::Pinecone.short_name(), "pinecone");
        assert_eq!(SourceType::Weaviate.short_name(), "weaviate");
        assert_eq!(SourceType::Milvus.short_name(), "milvus");
        assert_eq!(SourceType::ChromaDB.short_name(), "chromadb");
        assert_eq!(SourceType::PgVector.short_name(), "pgvector");
    }

    #[test]
    fn test_source_type_requires_api_key() {
        // Sources that REQUIRE API key
        assert!(SourceType::Supabase.requires_api_key());
        assert!(SourceType::Pinecone.requires_api_key());

        // Sources that do NOT require API key
        assert!(!SourceType::Qdrant.requires_api_key());
        assert!(!SourceType::Weaviate.requires_api_key());
        assert!(!SourceType::Milvus.requires_api_key());
        assert!(!SourceType::ChromaDB.requires_api_key());
        assert!(!SourceType::PgVector.requires_api_key());
    }

    #[test]
    fn test_source_type_optional_api_key() {
        // Sources with OPTIONAL API key
        assert!(SourceType::Qdrant.optional_api_key());
        assert!(SourceType::Weaviate.optional_api_key());
        assert!(SourceType::Milvus.optional_api_key());

        // Sources without optional API key
        assert!(!SourceType::Supabase.optional_api_key());
        assert!(!SourceType::Pinecone.optional_api_key());
        assert!(!SourceType::ChromaDB.optional_api_key());
        assert!(!SourceType::PgVector.optional_api_key());
    }

    // ==================== WizardConfig Tests ====================

    #[test]
    fn test_wizard_config_creation() {
        // Arrange & Act
        let config = WizardConfig {
            source_type: SourceType::Qdrant,
            url: "http://localhost:6333".to_string(),
            api_key: None,
            collection: "test_collection".to_string(),
            dest_path: "./velesdb_data".to_string(),
            use_sq8: false,
        };

        // Assert
        assert_eq!(config.source_type, SourceType::Qdrant);
        assert_eq!(config.url, "http://localhost:6333");
        assert!(config.api_key.is_none());
        assert_eq!(config.collection, "test_collection");
        assert_eq!(config.dest_path, "./velesdb_data");
        assert!(!config.use_sq8);
    }

    #[test]
    fn test_wizard_config_with_api_key() {
        // Arrange & Act
        let config = WizardConfig {
            source_type: SourceType::Supabase,
            url: "https://xyz.supabase.co".to_string(),
            api_key: Some("secret-key".to_string()),
            collection: "documents".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: true,
        };

        // Assert
        assert_eq!(config.source_type, SourceType::Supabase);
        assert_eq!(config.api_key, Some("secret-key".to_string()));
        assert!(config.use_sq8);
    }

    // ==================== Wizard Tests ====================

    #[test]
    fn test_wizard_new() {
        // Arrange & Act
        let wizard = Wizard::new();

        // Assert - wizard should be created without panic
        // We can't easily test internal state, but creation should work
        drop(wizard);
    }

    #[test]
    fn test_wizard_default() {
        // Arrange & Act
        let wizard = Wizard::default();

        // Assert
        drop(wizard);
    }

    // ==================== Build Source Config Tests ====================

    #[test]
    fn test_build_source_config_qdrant() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::Qdrant,
            url: "http://localhost:6333".to_string(),
            api_key: Some("test-key".to_string()),
            collection: "vectors".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: false,
        };

        // Act
        let result = wizard.build_source_config(&config);

        // Assert
        assert!(result.is_ok());
        let source = result.unwrap();
        match source {
            SourceConfig::Qdrant(cfg) => {
                assert_eq!(cfg.url, "http://localhost:6333");
                assert_eq!(cfg.collection, "vectors");
                assert_eq!(cfg.api_key, Some("test-key".to_string()));
            }
            _ => panic!("Expected Qdrant config"),
        }
    }

    #[test]
    fn test_build_source_config_supabase() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::Supabase,
            url: "https://xyz.supabase.co".to_string(),
            api_key: Some("service-key".to_string()),
            collection: "documents".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: false,
        };

        // Act
        let result = wizard.build_source_config(&config);

        // Assert
        assert!(result.is_ok());
        let source = result.unwrap();
        match source {
            SourceConfig::Supabase(cfg) => {
                assert_eq!(cfg.url, "https://xyz.supabase.co");
                assert_eq!(cfg.table, "documents");
                assert_eq!(cfg.api_key, "service-key");
            }
            _ => panic!("Expected Supabase config"),
        }
    }

    #[test]
    fn test_build_source_config_chromadb() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::ChromaDB,
            url: "http://localhost:8000".to_string(),
            api_key: None,
            collection: "embeddings".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: false,
        };

        // Act
        let result = wizard.build_source_config(&config);

        // Assert
        assert!(result.is_ok());
        let source = result.unwrap();
        match source {
            SourceConfig::ChromaDB(cfg) => {
                assert_eq!(cfg.url, "http://localhost:8000");
                assert_eq!(cfg.collection, "embeddings");
            }
            _ => panic!("Expected ChromaDB config"),
        }
    }

    #[test]
    fn test_build_source_config_pinecone() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::Pinecone,
            url: "https://index.pinecone.io".to_string(),
            api_key: Some("pinecone-key".to_string()),
            collection: "my-index".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: false,
        };

        // Act
        let result = wizard.build_source_config(&config);

        // Assert
        assert!(result.is_ok());
        let source = result.unwrap();
        match source {
            SourceConfig::Pinecone(cfg) => {
                assert_eq!(cfg.api_key, "pinecone-key");
                assert_eq!(cfg.index, "my-index");
            }
            _ => panic!("Expected Pinecone config"),
        }
    }

    #[test]
    fn test_build_source_config_weaviate() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::Weaviate,
            url: "http://localhost:8080".to_string(),
            api_key: None,
            collection: "Document".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: false,
        };

        // Act
        let result = wizard.build_source_config(&config);

        // Assert
        assert!(result.is_ok());
        let source = result.unwrap();
        match source {
            SourceConfig::Weaviate(cfg) => {
                assert_eq!(cfg.url, "http://localhost:8080");
                assert_eq!(cfg.class_name, "Document");
            }
            _ => panic!("Expected Weaviate config"),
        }
    }

    #[test]
    fn test_build_source_config_milvus() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::Milvus,
            url: "http://localhost:19530".to_string(),
            api_key: None,
            collection: "vectors".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: false,
        };

        // Act
        let result = wizard.build_source_config(&config);

        // Assert
        assert!(result.is_ok());
        let source = result.unwrap();
        match source {
            SourceConfig::Milvus(cfg) => {
                assert_eq!(cfg.url, "http://localhost:19530");
                assert_eq!(cfg.collection, "vectors");
            }
            _ => panic!("Expected Milvus config"),
        }
    }

    // ==================== Build Migration Config Tests ====================

    #[test]
    fn test_build_migration_config_full_storage() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::Qdrant,
            url: "http://localhost:6333".to_string(),
            api_key: None,
            collection: "test".to_string(),
            dest_path: "./velesdb_data".to_string(),
            use_sq8: false,
        };
        let schema = crate::connectors::SourceSchema {
            source_type: "qdrant".to_string(),
            collection: "test".to_string(),
            dimension: 768,
            total_count: Some(1000),
            fields: vec![],
            vector_column: None,
            id_column: None,
        };

        // Act
        let result = wizard.build_migration_config(&config, &schema);

        // Assert
        assert!(result.is_ok());
        let migration = result.unwrap();
        assert_eq!(migration.destination.dimension, 768);
        assert_eq!(migration.destination.collection, "test");
        assert!(matches!(
            migration.destination.storage_mode,
            StorageMode::Full
        ));
        assert!(matches!(
            migration.destination.metric,
            DistanceMetric::Cosine
        ));
    }

    #[test]
    fn test_build_migration_config_sq8_storage() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::Qdrant,
            url: "http://localhost:6333".to_string(),
            api_key: None,
            collection: "test".to_string(),
            dest_path: "./velesdb_data".to_string(),
            use_sq8: true, // SQ8 enabled
        };
        let schema = crate::connectors::SourceSchema {
            source_type: "qdrant".to_string(),
            collection: "test".to_string(),
            dimension: 1536,
            total_count: Some(5000),
            fields: vec![],
            vector_column: None,
            id_column: None,
        };

        // Act
        let result = wizard.build_migration_config(&config, &schema);

        // Assert
        assert!(result.is_ok());
        let migration = result.unwrap();
        assert_eq!(migration.destination.dimension, 1536);
        assert!(matches!(
            migration.destination.storage_mode,
            StorageMode::SQ8
        ));
    }

    #[test]
    fn test_build_migration_config_options() {
        // Arrange
        let wizard = Wizard::new();
        let config = WizardConfig {
            source_type: SourceType::ChromaDB,
            url: "http://localhost:8000".to_string(),
            api_key: None,
            collection: "embeddings".to_string(),
            dest_path: "./data".to_string(),
            use_sq8: false,
        };
        let schema = crate::connectors::SourceSchema {
            source_type: "chromadb".to_string(),
            collection: "embeddings".to_string(),
            dimension: 384,
            total_count: None,
            fields: vec![],
            vector_column: None,
            id_column: None,
        };

        // Act
        let result = wizard.build_migration_config(&config, &schema);

        // Assert
        assert!(result.is_ok());
        let migration = result.unwrap();
        assert_eq!(migration.options.batch_size, 1000);
        assert_eq!(migration.options.workers, 4);
        assert!(!migration.options.dry_run);
        assert!(migration.options.checkpoint_enabled);
    }
}
