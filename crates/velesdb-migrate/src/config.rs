//! Configuration types for velesdb-migrate.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Main migration configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// Source database configuration.
    pub source: SourceConfig,
    /// Destination `VelesDB` configuration.
    pub destination: DestinationConfig,
    /// Migration options.
    #[serde(default)]
    pub options: MigrationOptions,
}

/// Source database configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    /// `PostgreSQL` with pgvector extension.
    #[serde(rename = "pgvector")]
    PgVector(PgVectorConfig),
    /// Supabase (pgvector-based).
    #[serde(rename = "supabase")]
    Supabase(SupabaseConfig),
    /// Qdrant vector database.
    #[serde(rename = "qdrant")]
    Qdrant(QdrantConfig),
    /// Pinecone vector database.
    #[serde(rename = "pinecone")]
    Pinecone(PineconeConfig),
    /// Weaviate vector database.
    #[serde(rename = "weaviate")]
    Weaviate(WeaviateConfig),
    /// Milvus vector database.
    #[serde(rename = "milvus")]
    Milvus(MilvusConfig),
    /// `ChromaDB` vector database.
    #[serde(rename = "chromadb")]
    ChromaDB(ChromaDBConfig),
    /// JSON file import.
    #[serde(rename = "json_file")]
    JsonFile(crate::connectors::json_file::JsonFileConfig),
    /// CSV file import.
    #[serde(rename = "csv_file")]
    CsvFile(crate::connectors::csv_file::CsvFileConfig),
    /// MongoDB Atlas Vector Search.
    #[serde(rename = "mongodb")]
    MongoDB(crate::connectors::mongodb::MongoDBConfig),
    /// Elasticsearch/OpenSearch with vector search.
    #[serde(rename = "elasticsearch")]
    Elasticsearch(crate::connectors::elasticsearch::ElasticsearchConfig),
    /// Redis Vector Search (Redis Stack).
    #[serde(rename = "redis")]
    Redis(crate::connectors::redis::RedisConfig),
}

/// `PostgreSQL` pgvector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgVectorConfig {
    /// Connection string (postgres://user:pass@host:port/db).
    pub connection_string: String,
    /// Table name containing vectors.
    pub table: String,
    /// Column name for vector data.
    #[serde(default = "default_vector_column")]
    pub vector_column: String,
    /// Column name for primary key/ID.
    #[serde(default = "default_id_column")]
    pub id_column: String,
    /// Additional columns to include in payload.
    #[serde(default)]
    pub payload_columns: Vec<String>,
    /// Optional WHERE clause for filtering.
    pub filter: Option<String>,
}

/// Supabase configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupabaseConfig {
    /// Supabase project URL.
    pub url: String,
    /// Supabase service role key or anon key.
    pub api_key: String,
    /// Table name containing vectors.
    pub table: String,
    /// Column name for vector data.
    #[serde(default = "default_vector_column")]
    pub vector_column: String,
    /// Column name for primary key/ID.
    #[serde(default = "default_id_column")]
    pub id_column: String,
    /// Additional columns to include in payload.
    #[serde(default)]
    pub payload_columns: Vec<String>,
}

/// Qdrant configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantConfig {
    /// Qdrant server URL.
    pub url: String,
    /// Collection name.
    pub collection: String,
    /// Optional API key.
    pub api_key: Option<String>,
    /// Include payload fields (empty = all).
    #[serde(default)]
    pub payload_fields: Vec<String>,
}

/// Pinecone configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PineconeConfig {
    /// Pinecone API key.
    pub api_key: String,
    /// Environment (e.g., "us-east-1-aws").
    pub environment: String,
    /// Index name.
    pub index: String,
    /// Optional namespace.
    pub namespace: Option<String>,
}

/// Weaviate configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeaviateConfig {
    /// Weaviate server URL.
    pub url: String,
    /// Class name.
    pub class_name: String,
    /// Optional API key.
    pub api_key: Option<String>,
    /// Properties to include.
    #[serde(default)]
    pub properties: Vec<String>,
}

/// Milvus configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilvusConfig {
    /// Milvus server URL.
    pub url: String,
    /// Collection name.
    pub collection: String,
    /// Optional username.
    pub username: Option<String>,
    /// Optional password.
    pub password: Option<String>,
}

/// `ChromaDB` configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromaDBConfig {
    /// `ChromaDB` server URL.
    pub url: String,
    /// Collection name.
    pub collection: String,
    /// Optional tenant.
    pub tenant: Option<String>,
    /// Optional database.
    pub database: Option<String>,
}

/// Destination `VelesDB` configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationConfig {
    /// Path to `VelesDB` database directory.
    pub path: PathBuf,
    /// Collection name (will be created if not exists).
    pub collection: String,
    /// Vector dimension (must match source).
    pub dimension: usize,
    /// Distance metric.
    #[serde(default)]
    pub metric: DistanceMetric,
    /// Storage mode.
    #[serde(default)]
    pub storage_mode: StorageMode,
}

/// Distance metric for `VelesDB`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistanceMetric {
    /// Cosine similarity (default). Best for normalized embeddings.
    #[default]
    Cosine,
    /// Euclidean distance. Best for unnormalized embeddings.
    Euclidean,
    /// Dot product. Fast but requires normalized vectors.
    Dot,
}

/// Storage mode for `VelesDB`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    /// Full precision (32-bit float). No compression.
    #[default]
    Full,
    /// Scalar quantization (8-bit). 4x compression, ~99% recall.
    SQ8,
    /// Binary quantization (1-bit). 32x compression, ~95% recall.
    Binary,
}

/// Migration options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationOptions {
    /// Batch size for extraction and loading.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Enable checkpoint/resume support.
    #[serde(default = "default_true")]
    pub checkpoint_enabled: bool,
    /// Checkpoint file path.
    pub checkpoint_path: Option<PathBuf>,
    /// Number of parallel workers.
    #[serde(default = "default_workers")]
    pub workers: usize,
    /// Dry run mode (don't write to destination).
    #[serde(default)]
    pub dry_run: bool,
    /// Field mappings (`source_field` -> `dest_field`).
    #[serde(default)]
    pub field_mappings: HashMap<String, String>,
    /// Continue on errors.
    #[serde(default)]
    pub continue_on_error: bool,
}

impl Default for MigrationOptions {
    fn default() -> Self {
        Self {
            batch_size: default_batch_size(),
            checkpoint_enabled: true,
            checkpoint_path: None,
            workers: default_workers(),
            dry_run: false,
            field_mappings: HashMap::new(),
            continue_on_error: false,
        }
    }
}

fn default_vector_column() -> String {
    "embedding".to_string()
}

fn default_id_column() -> String {
    "id".to_string()
}

fn default_batch_size() -> usize {
    1000
}

fn default_workers() -> usize {
    4
}

fn default_true() -> bool {
    true
}

impl MigrationConfig {
    /// Load configuration from a YAML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn from_file(path: &std::path::Path) -> crate::error::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Validate the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.destination.dimension == 0 {
            return Err(crate::error::Error::Config(
                "dimension must be greater than 0".to_string(),
            ));
        }
        if self.destination.collection.is_empty() {
            return Err(crate::error::Error::Config(
                "collection name cannot be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let options = MigrationOptions::default();
        assert_eq!(options.batch_size, 1000);
        assert_eq!(options.workers, 4);
        assert!(options.checkpoint_enabled);
        assert!(!options.dry_run);
    }

    #[test]
    fn test_config_validate_dimension() {
        let config = MigrationConfig {
            source: SourceConfig::Qdrant(QdrantConfig {
                url: "http://localhost:6333".to_string(),
                collection: "test".to_string(),
                api_key: None,
                payload_fields: vec![],
            }),
            destination: DestinationConfig {
                path: PathBuf::from("./test_db"),
                collection: "test".to_string(),
                dimension: 0,
                metric: DistanceMetric::Cosine,
                storage_mode: StorageMode::Full,
            },
            options: MigrationOptions::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_config_yaml_parse() {
        let yaml = r#"
source:
  type: qdrant
  url: http://localhost:6333
  collection: documents
destination:
  path: ./velesdb_data
  collection: docs
  dimension: 768
options:
  batch_size: 500
"#;
        let config: MigrationConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.destination.dimension, 768);
        assert_eq!(config.options.batch_size, 500);
    }
}
