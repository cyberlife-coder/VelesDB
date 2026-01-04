//! Auto-discovery of collections and tables from source databases.

use crate::config::SourceConfig;
use crate::connectors::{create_connector, SourceSchema};
use crate::error::Result;

use super::SourceType;

/// Discovered collection information.
#[derive(Debug, Clone)]
pub struct DiscoveredCollection {
    /// Collection/table name.
    pub name: String,
    /// Vector count (if available).
    pub count: Option<u64>,
    /// Vector dimension (if available).
    pub dimension: usize,
}

/// Source discovery utilities.
pub struct SourceDiscovery;

impl SourceDiscovery {
    /// Lists available collections from a source.
    ///
    /// Note: Currently returns only the specified collection's schema.
    /// Future versions will support listing all collections.
    pub async fn list_collections(
        source_type: SourceType,
        url: &str,
        api_key: Option<&str>,
        collection: &str,
    ) -> Result<Vec<DiscoveredCollection>> {
        let source_config = Self::build_config(source_type, url, api_key, collection)?;
        let mut connector = create_connector(&source_config)?;

        connector.connect().await?;
        let schema = connector.get_schema().await?;
        connector.close().await?;

        Ok(vec![DiscoveredCollection {
            name: schema.collection.clone(),
            count: schema.total_count,
            dimension: schema.dimension,
        }])
    }

    /// Gets schema for a specific collection.
    pub async fn get_schema(
        source_type: SourceType,
        url: &str,
        api_key: Option<&str>,
        collection: &str,
    ) -> Result<SourceSchema> {
        let source_config = Self::build_config(source_type, url, api_key, collection)?;
        let mut connector = create_connector(&source_config)?;

        connector.connect().await?;
        let schema = connector.get_schema().await?;
        connector.close().await?;

        Ok(schema)
    }

    /// Builds source config for discovery.
    fn build_config(
        source_type: SourceType,
        url: &str,
        api_key: Option<&str>,
        collection: &str,
    ) -> Result<SourceConfig> {
        use crate::config::*;

        let config = match source_type {
            SourceType::Supabase => SourceConfig::Supabase(SupabaseConfig {
                url: url.to_string(),
                api_key: api_key.unwrap_or_default().to_string(),
                table: collection.to_string(),
                vector_column: "embedding".to_string(),
                id_column: "id".to_string(),
                payload_columns: vec![],
            }),
            SourceType::Qdrant => SourceConfig::Qdrant(QdrantConfig {
                url: url.to_string(),
                collection: collection.to_string(),
                api_key: api_key.map(String::from),
                payload_fields: vec![],
            }),
            SourceType::Pinecone => SourceConfig::Pinecone(PineconeConfig {
                api_key: api_key.unwrap_or_default().to_string(),
                environment: String::new(),
                index: collection.to_string(),
                namespace: None,
            }),
            SourceType::Weaviate => SourceConfig::Weaviate(WeaviateConfig {
                url: url.to_string(),
                class_name: collection.to_string(),
                api_key: api_key.map(String::from),
                properties: vec![],
            }),
            SourceType::Milvus => SourceConfig::Milvus(MilvusConfig {
                url: url.to_string(),
                collection: collection.to_string(),
                username: None,
                password: None,
            }),
            SourceType::ChromaDB => SourceConfig::ChromaDB(ChromaDBConfig {
                url: url.to_string(),
                collection: collection.to_string(),
                tenant: None,
                database: None,
            }),
            SourceType::PgVector => {
                #[cfg(feature = "postgres")]
                {
                    SourceConfig::PgVector(PgVectorConfig {
                        connection_string: url.to_string(),
                        table: collection.to_string(),
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
                    path: std::path::PathBuf::from(url),
                    array_path: String::new(),
                    id_field: "id".to_string(),
                    vector_field: "vector".to_string(),
                    payload_fields: vec![],
                })
            }
            SourceType::CsvFile => {
                SourceConfig::CsvFile(crate::connectors::csv_file::CsvFileConfig {
                    path: std::path::PathBuf::from(url),
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
                    data_api_url: url.to_string(),
                    api_key: api_key.unwrap_or_default().to_string(),
                    database: "vectors".to_string(),
                    collection: collection.to_string(),
                    vector_field: "embedding".to_string(),
                    id_field: "_id".to_string(),
                    payload_fields: vec![],
                    filter: None,
                })
            }
            SourceType::Elasticsearch => {
                SourceConfig::Elasticsearch(crate::connectors::elasticsearch::ElasticsearchConfig {
                    url: url.to_string(),
                    index: collection.to_string(),
                    vector_field: "embedding".to_string(),
                    id_field: "_id".to_string(),
                    payload_fields: vec![],
                    username: None,
                    password: None,
                    api_key: api_key.map(String::from),
                    query: None,
                })
            }
            SourceType::Redis => SourceConfig::Redis(crate::connectors::redis::RedisConfig {
                url: url.to_string(),
                password: api_key.map(String::from),
                index: collection.to_string(),
                vector_field: "embedding".to_string(),
                key_prefix: "doc:".to_string(),
                payload_fields: vec![],
                filter: None,
            }),
        };

        Ok(config)
    }
}
