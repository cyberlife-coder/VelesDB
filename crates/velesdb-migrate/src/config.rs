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
    /// Relations to migrate as graph edges (optional).
    #[serde(default)]
    pub relations: Vec<RelationConfig>,
}

/// Source database configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    /// Supabase (PostgREST API over a pgvector-enabled PostgreSQL).
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
    /// Elasticsearch/OpenSearch with vector search.
    #[serde(rename = "elasticsearch")]
    Elasticsearch(crate::connectors::elasticsearch::ElasticsearchConfig),
    /// Redis Vector Search (Redis Stack).
    #[serde(rename = "redis")]
    Redis(RedisConfig),
}

/// Configuration for Redis Vector Search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// Redis URL (e.g., `redis://localhost:6379` or `rediss://...` for TLS).
    pub url: String,
    /// Redis password (optional).
    #[serde(default)]
    pub password: Option<String>,
    /// Index name created with `FT.CREATE`.
    pub index: String,
    /// Field name containing the vector embedding.
    #[serde(default = "default_redis_vector_field")]
    pub vector_field: String,
    /// Prefix for document keys (e.g., "doc:").
    #[serde(default = "default_redis_key_prefix")]
    pub key_prefix: String,
    /// Fields to include in payload (empty = all).
    #[serde(default)]
    pub payload_fields: Vec<String>,
    /// Optional filter query (RediSearch syntax).
    #[serde(default)]
    pub filter: Option<String>,
}

fn default_redis_vector_field() -> String {
    "embedding".to_string()
}

fn default_redis_key_prefix() -> String {
    "doc:".to_string()
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
    /// Optional distance metric declared by the operator.
    ///
    /// Supabase's PostgREST surface does not expose `pg_catalog`
    /// tables by default, so the pgvector index operator class
    /// (`vector_cosine_ops`, `vector_l2_ops`, `vector_ip_ops`)
    /// cannot be auto-introspected without a custom RPC. Operators
    /// who know their index definition can declare it here and the
    /// value is forwarded to `SourceSchema.metric` so
    /// `Pipeline::check_metric_fidelity` can catch mismatches.
    ///
    /// Accepted values: the VelesDB core vocabulary
    /// (`cosine`/`euclidean`/`dot`/...) or the pgvector operator
    /// class aliases (`vector_cosine_ops`/`vector_l2_ops`/
    /// `vector_ip_ops`). Values are normalised before forwarding.
    /// Leaving this unset emits a `tracing::warn!` on `get_schema`
    /// so the skipped fidelity check is never silent.
    #[serde(default)]
    pub metric: Option<String>,
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
#[allow(deprecated)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PineconeConfig {
    /// Pinecone API key.
    pub api_key: String,
    /// Deprecated: Pinecone serverless (2024+) discovers the host dynamically
    /// via `GET /indexes/{name}`. Kept for backward compatibility with existing YAML configs.
    #[serde(default)]
    #[deprecated(
        since = "1.12.0",
        note = "Pinecone serverless ignores environments; host is discovered via the API"
    )]
    pub environment: String,
    /// Index name.
    pub index: String,
    /// Optional namespace.
    pub namespace: Option<String>,
    /// Override base URL for testing (replaces `https://api.pinecone.io`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
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

/// Configuration of a source relation to migrate as graph edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationConfig {
    /// Column/field in the source containing the FK (e.g., "author_id").
    pub from_column: String,
    /// Target table/collection (e.g., "authors").
    pub to_table: String,
    /// ID column in the target (e.g., "id"). Defaults to "id".
    #[serde(default = "default_relation_id_column")]
    pub to_column: String,
    /// Edge label in `VelesDB` (e.g., "AUTHORED_BY").
    pub edge_label: String,
    /// Optional column for a numeric edge weight.
    #[serde(default)]
    pub weight_column: Option<String>,
}

fn default_relation_id_column() -> String {
    "id".to_string()
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
    /// Name of the `GraphCollection` for graph edges (optional).
    #[serde(default)]
    pub graph_collection: Option<String>,
}

/// Distance metric for `VelesDB`.
///
/// This enum is the migrate TOML config schema. Its accepted spellings and
/// aliases are backward-compatibility surface and must not change. The mapping
/// to the core type is authoritative-from-core: each variant exposes its
/// core-canonical name via [`DistanceMetric::core_name`] and the [`From`] impl
/// resolves it through [`velesdb_core::DistanceMetric::parse_alias`] (the
/// single source of truth) instead of a hand-maintained match.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistanceMetric {
    /// Cosine similarity (default). Best for normalized embeddings.
    #[default]
    Cosine,
    /// Euclidean distance. Best for unnormalized embeddings.
    Euclidean,
    /// Dot product. Fast but requires normalized vectors.
    #[serde(alias = "DotProduct", alias = "dot_product")]
    Dot,
    /// Hamming distance for binary vectors.
    Hamming,
    /// Jaccard similarity for set-like vectors.
    Jaccard,
}

impl DistanceMetric {
    /// Core-canonical name accepted by
    /// [`velesdb_core::DistanceMetric::parse_alias`].
    #[must_use]
    pub const fn core_name(self) -> &'static str {
        match self {
            Self::Cosine => "cosine",
            Self::Euclidean => "euclidean",
            Self::Dot => "dot",
            Self::Hamming => "hamming",
            Self::Jaccard => "jaccard",
        }
    }
}

impl From<DistanceMetric> for velesdb_core::DistanceMetric {
    fn from(m: DistanceMetric) -> Self {
        // Authoritative-from-core: every `core_name` is a known core alias, so
        // this is infallible. The `Cosine` fallback keeps the conversion total
        // without an `unwrap`; a `core_name` regression is caught by the unit
        // tests (`test_distance_metric_aliases_deserialize_and_map_to_core`).
        velesdb_core::DistanceMetric::parse_alias(m.core_name())
            .unwrap_or(velesdb_core::DistanceMetric::Cosine)
    }
}

/// Storage mode for `VelesDB`.
///
/// This enum is the migrate TOML config schema. Its accepted spellings and
/// aliases are backward-compatibility surface and must not change. The mapping
/// to the core type is authoritative-from-core: each variant exposes its
/// core-canonical name via [`StorageMode::core_name`] and the [`From`] impl
/// resolves it through [`velesdb_core::StorageMode::parse_alias`] (the single
/// source of truth) instead of a hand-maintained match.
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
    /// Product quantization. High compression with trained codebooks.
    #[serde(alias = "product_quantization")]
    Pq,
    /// `RaBitQ`: 1-bit with rotation + scalar correction. 32x compression.
    #[serde(alias = "rabitq")]
    RaBitQ,
}

impl StorageMode {
    /// Core-canonical name accepted by
    /// [`velesdb_core::StorageMode::parse_alias`].
    #[must_use]
    pub const fn core_name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::SQ8 => "sq8",
            Self::Binary => "binary",
            Self::Pq => "pq",
            Self::RaBitQ => "rabitq",
        }
    }
}

impl From<StorageMode> for velesdb_core::StorageMode {
    fn from(m: StorageMode) -> Self {
        // Authoritative-from-core: every `core_name` is a known core alias, so
        // this is infallible. The `Full` fallback keeps the conversion total
        // without an `unwrap`; a `core_name` regression is caught by the unit
        // tests (`test_storage_mode_aliases_deserialize_and_map_to_core`).
        velesdb_core::StorageMode::parse_alias(m.core_name())
            .unwrap_or(velesdb_core::StorageMode::Full)
    }
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
    /// Allow the pipeline to proceed even when the source reports a
    /// distance metric that differs from the destination
    /// configuration (finding M-P0-3). Defaults to `false`: a
    /// mismatch aborts the migration with an error that names both
    /// metrics. Set to `true` for controlled migrations where the
    /// operator knows the semantic difference is acceptable.
    #[serde(default)]
    pub allow_metric_mismatch: bool,
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
            allow_metric_mismatch: false,
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
        if self.options.batch_size == 0 {
            return Err(crate::error::Error::Config(
                "batch_size must be greater than 0".to_string(),
            ));
        }
        if self.options.workers == 0 {
            return Err(crate::error::Error::Config(
                "workers must be greater than 0".to_string(),
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
#[path = "config_tests.rs"]
mod tests;
