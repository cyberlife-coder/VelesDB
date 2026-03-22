//! Canonical builder for `SourceConfig` from basic connection parameters.
//!
//! This module eliminates the triple-duplication of source config construction
//! across the wizard, discovery, and CLI modules. Each call site previously
//! maintained its own 100+ line match block mapping source types to config
//! variants with identical defaults. Now they all delegate here.

use crate::config::SourceConfig;
use crate::error::{Error, Result};
use crate::wizard::SourceType;

/// Minimal connection parameters needed to build a `SourceConfig`.
///
/// This struct unifies the three different calling conventions:
/// - Wizard: has `WizardConfig` with `source_type: SourceType`
/// - Discovery: has `(SourceType, &str, Option<&str>, &str)`
/// - CLI detect: has `(&str, &str, &str, Option<&str>)`
#[derive(Debug, Clone)]
pub struct SourceParams<'a> {
    /// Source type identifier.
    pub source_type: SourceType,
    /// URL or connection string.
    pub url: &'a str,
    /// Optional API key.
    pub api_key: Option<&'a str>,
    /// Collection/table/index name.
    pub collection: &'a str,
}

/// Builds a `SourceConfig` from basic connection parameters.
///
/// Uses sensible defaults for fields not provided (e.g., vector column
/// defaults to "embedding", id column defaults to "id").
///
/// # Errors
///
/// Returns `Error::Config` if:
/// - The source type requires a feature flag that is not enabled (e.g.,
///   pgvector requires `--features postgres`).
/// - A required API key is missing or empty (Supabase, Pinecone, MongoDB).
pub fn build_source_config(params: &SourceParams<'_>) -> Result<SourceConfig> {
    match params.source_type {
        SourceType::Supabase => build_supabase(params),
        SourceType::Qdrant => Ok(build_qdrant(params)),
        SourceType::Pinecone => build_pinecone(params),
        SourceType::Weaviate => Ok(build_weaviate(params)),
        SourceType::Milvus => Ok(build_milvus(params)),
        SourceType::ChromaDB => Ok(build_chromadb(params)),
        SourceType::PgVector => build_pgvector(params),
        SourceType::JsonFile => Ok(build_json_file(params)),
        SourceType::CsvFile => Ok(build_csv_file(params)),
        SourceType::MongoDB => build_mongodb(params),
        SourceType::Elasticsearch => Ok(build_elasticsearch(params)),
        SourceType::Redis => Ok(build_redis(params)),
    }
}

/// Extracts and validates a required API key from source parameters.
///
/// # Errors
///
/// Returns `Error::Config` if the API key is `None` or empty.
fn require_api_key(params: &SourceParams<'_>, source_name: &str) -> Result<String> {
    params
        .api_key
        .filter(|k| !k.is_empty())
        .map(String::from)
        .ok_or_else(|| Error::Config(format!("{source_name} requires an API key (--api-key)")))
}

fn build_supabase(params: &SourceParams<'_>) -> Result<SourceConfig> {
    let api_key = require_api_key(params, "Supabase")?;
    Ok(SourceConfig::Supabase(crate::config::SupabaseConfig {
        url: params.url.to_string(),
        api_key,
        table: params.collection.to_string(),
        vector_column: "embedding".to_string(),
        id_column: "id".to_string(),
        payload_columns: vec![],
    }))
}

fn build_qdrant(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Qdrant(crate::config::QdrantConfig {
        url: params.url.to_string(),
        collection: params.collection.to_string(),
        api_key: params.api_key.map(String::from),
        payload_fields: vec![],
    })
}

fn build_pinecone(params: &SourceParams<'_>) -> Result<SourceConfig> {
    let api_key = require_api_key(params, "Pinecone")?;
    Ok(SourceConfig::Pinecone(crate::config::PineconeConfig {
        api_key,
        environment: String::new(),
        index: params.collection.to_string(),
        namespace: None,
        base_url: None,
    }))
}

fn build_weaviate(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Weaviate(crate::config::WeaviateConfig {
        url: params.url.to_string(),
        class_name: params.collection.to_string(),
        api_key: params.api_key.map(String::from),
        properties: vec![],
    })
}

fn build_milvus(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Milvus(crate::config::MilvusConfig {
        url: params.url.to_string(),
        collection: params.collection.to_string(),
        username: None,
        password: None,
    })
}

fn build_chromadb(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::ChromaDB(crate::config::ChromaDBConfig {
        url: params.url.to_string(),
        collection: params.collection.to_string(),
        tenant: None,
        database: None,
    })
}

fn build_pgvector(params: &SourceParams<'_>) -> Result<SourceConfig> {
    #[cfg(feature = "postgres")]
    {
        Ok(SourceConfig::PgVector(crate::config::PgVectorConfig {
            connection_string: params.url.to_string(),
            table: params.collection.to_string(),
            vector_column: "embedding".to_string(),
            id_column: "id".to_string(),
            payload_columns: vec![],
            filter: None,
        }))
    }
    #[cfg(not(feature = "postgres"))]
    {
        let _ = params;
        Err(Error::Config(
            "pgvector requires --features postgres".to_string(),
        ))
    }
}

fn build_json_file(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::JsonFile(crate::connectors::json_file::JsonFileConfig {
        path: std::path::PathBuf::from(params.url),
        array_path: String::new(),
        id_field: "id".to_string(),
        vector_field: "vector".to_string(),
        payload_fields: vec![],
    })
}

fn build_csv_file(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::CsvFile(crate::connectors::csv_file::CsvFileConfig {
        path: std::path::PathBuf::from(params.url),
        id_column: "id".to_string(),
        vector_column: "vector".to_string(),
        vector_spread: false,
        dim_prefix: "dim_".to_string(),
        delimiter: ',',
        has_header: true,
    })
}

fn build_mongodb(params: &SourceParams<'_>) -> Result<SourceConfig> {
    let api_key = require_api_key(params, "MongoDB")?;
    Ok(SourceConfig::MongoDB(
        crate::connectors::mongodb::MongoDBConfig {
            data_api_url: params.url.to_string(),
            api_key,
            database: "vectors".to_string(),
            collection: params.collection.to_string(),
            vector_field: "embedding".to_string(),
            id_field: "_id".to_string(),
            payload_fields: vec![],
            filter: None,
        },
    ))
}

fn build_elasticsearch(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Elasticsearch(crate::connectors::elasticsearch::ElasticsearchConfig {
        url: params.url.to_string(),
        index: params.collection.to_string(),
        vector_field: "embedding".to_string(),
        id_field: "_id".to_string(),
        payload_fields: vec![],
        username: None,
        password: None,
        api_key: params.api_key.map(String::from),
        query: None,
    })
}

fn build_redis(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Redis(crate::connectors::redis::RedisConfig {
        url: params.url.to_string(),
        password: params.api_key.map(String::from),
        index: params.collection.to_string(),
        vector_field: "embedding".to_string(),
        key_prefix: "doc:".to_string(),
        payload_fields: vec![],
        filter: None,
    })
}

/// Parses a source type string into a `SourceType`.
///
/// # Errors
///
/// Returns `Error::Config` if the source type string is not recognized.
pub fn parse_source_type(source_type: &str) -> Result<SourceType> {
    match source_type.to_lowercase().as_str() {
        "supabase" => Ok(SourceType::Supabase),
        "qdrant" => Ok(SourceType::Qdrant),
        "pinecone" => Ok(SourceType::Pinecone),
        "weaviate" => Ok(SourceType::Weaviate),
        "milvus" => Ok(SourceType::Milvus),
        "chromadb" => Ok(SourceType::ChromaDB),
        "pgvector" => Ok(SourceType::PgVector),
        "json_file" | "json" => Ok(SourceType::JsonFile),
        "csv_file" | "csv" => Ok(SourceType::CsvFile),
        "mongodb" => Ok(SourceType::MongoDB),
        "elasticsearch" => Ok(SourceType::Elasticsearch),
        "redis" => Ok(SourceType::Redis),
        other => Err(Error::Config(format!("Unknown source type: {other}"))),
    }
}

/// Connects to a source, fetches its schema, and closes the connection.
///
/// This eliminates the repeated connect/get_schema/close pattern that
/// appeared in three places (wizard, discovery list, discovery get_schema).
///
/// # Errors
///
/// Returns the first error from connect, get_schema, or close.
pub async fn fetch_schema(source_config: &SourceConfig) -> Result<crate::connectors::SourceSchema> {
    let mut connector = crate::connectors::create_connector(source_config)?;
    connector.connect().await?;
    let schema = connector.get_schema().await?;
    connector.close().await?;
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_source_type_all_variants() {
        let cases = [
            ("supabase", SourceType::Supabase),
            ("qdrant", SourceType::Qdrant),
            ("pinecone", SourceType::Pinecone),
            ("weaviate", SourceType::Weaviate),
            ("milvus", SourceType::Milvus),
            ("chromadb", SourceType::ChromaDB),
            ("pgvector", SourceType::PgVector),
            ("json_file", SourceType::JsonFile),
            ("json", SourceType::JsonFile),
            ("csv_file", SourceType::CsvFile),
            ("csv", SourceType::CsvFile),
            ("mongodb", SourceType::MongoDB),
            ("elasticsearch", SourceType::Elasticsearch),
            ("redis", SourceType::Redis),
        ];

        for (input, expected) in &cases {
            let result = parse_source_type(input);
            assert!(
                result.is_ok(),
                "parse_source_type({input:?}) should succeed"
            );
            assert_eq!(result.unwrap(), *expected);
        }
    }

    #[test]
    fn test_parse_source_type_case_insensitive() {
        assert_eq!(parse_source_type("QDRANT").unwrap(), SourceType::Qdrant);
        assert_eq!(parse_source_type("Supabase").unwrap(), SourceType::Supabase);
    }

    #[test]
    fn test_parse_source_type_unknown() {
        assert!(parse_source_type("unknown").is_err());
        assert!(parse_source_type("").is_err());
    }

    #[test]
    fn test_build_source_config_qdrant() {
        let params = SourceParams {
            source_type: SourceType::Qdrant,
            url: "http://localhost:6333",
            api_key: Some("test-key"),
            collection: "vectors",
        };

        let result = build_source_config(&params);
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
        let params = SourceParams {
            source_type: SourceType::Supabase,
            url: "https://xyz.supabase.co",
            api_key: Some("service-key"),
            collection: "documents",
        };

        let result = build_source_config(&params);
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
    fn test_build_source_config_pinecone() {
        let params = SourceParams {
            source_type: SourceType::Pinecone,
            url: "https://index.pinecone.io",
            api_key: Some("pinecone-key"),
            collection: "my-index",
        };

        let result = build_source_config(&params);
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
        let params = SourceParams {
            source_type: SourceType::Weaviate,
            url: "http://localhost:8080",
            api_key: None,
            collection: "Document",
        };

        let result = build_source_config(&params);
        assert!(result.is_ok());
        let source = result.unwrap();
        match source {
            SourceConfig::Weaviate(cfg) => {
                assert_eq!(cfg.url, "http://localhost:8080");
                assert_eq!(cfg.class_name, "Document");
                assert!(cfg.api_key.is_none());
            }
            _ => panic!("Expected Weaviate config"),
        }
    }

    #[test]
    fn test_build_source_config_milvus() {
        let params = SourceParams {
            source_type: SourceType::Milvus,
            url: "http://localhost:19530",
            api_key: None,
            collection: "vectors",
        };

        let result = build_source_config(&params);
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

    #[test]
    fn test_build_source_config_chromadb() {
        let params = SourceParams {
            source_type: SourceType::ChromaDB,
            url: "http://localhost:8000",
            api_key: None,
            collection: "embeddings",
        };

        let result = build_source_config(&params);
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
    fn test_build_source_config_json_file() {
        let params = SourceParams {
            source_type: SourceType::JsonFile,
            url: "./vectors.json",
            api_key: None,
            collection: "unused",
        };

        let result = build_source_config(&params);
        assert!(result.is_ok());
        match result.unwrap() {
            SourceConfig::JsonFile(cfg) => {
                assert_eq!(cfg.path, std::path::PathBuf::from("./vectors.json"));
                assert_eq!(cfg.id_field, "id");
                assert_eq!(cfg.vector_field, "vector");
            }
            _ => panic!("Expected JsonFile config"),
        }
    }

    #[test]
    fn test_build_source_config_csv_file() {
        let params = SourceParams {
            source_type: SourceType::CsvFile,
            url: "./vectors.csv",
            api_key: None,
            collection: "unused",
        };

        let result = build_source_config(&params);
        assert!(result.is_ok());
        match result.unwrap() {
            SourceConfig::CsvFile(cfg) => {
                assert_eq!(cfg.path, std::path::PathBuf::from("./vectors.csv"));
                assert_eq!(cfg.delimiter, ',');
                assert!(cfg.has_header);
            }
            _ => panic!("Expected CsvFile config"),
        }
    }

    #[test]
    fn test_build_source_config_mongodb() {
        let params = SourceParams {
            source_type: SourceType::MongoDB,
            url: "https://data.mongodb-api.com/v1",
            api_key: Some("mongo-key"),
            collection: "embeddings",
        };

        let result = build_source_config(&params);
        assert!(result.is_ok());
        match result.unwrap() {
            SourceConfig::MongoDB(cfg) => {
                assert_eq!(cfg.data_api_url, "https://data.mongodb-api.com/v1");
                assert_eq!(cfg.api_key, "mongo-key");
                assert_eq!(cfg.collection, "embeddings");
            }
            _ => panic!("Expected MongoDB config"),
        }
    }

    #[test]
    fn test_build_source_config_elasticsearch() {
        let params = SourceParams {
            source_type: SourceType::Elasticsearch,
            url: "http://localhost:9200",
            api_key: Some("es-key"),
            collection: "vectors",
        };

        let result = build_source_config(&params);
        assert!(result.is_ok());
        match result.unwrap() {
            SourceConfig::Elasticsearch(cfg) => {
                assert_eq!(cfg.url, "http://localhost:9200");
                assert_eq!(cfg.index, "vectors");
                assert_eq!(cfg.api_key, Some("es-key".to_string()));
            }
            _ => panic!("Expected Elasticsearch config"),
        }
    }

    #[test]
    fn test_build_source_config_redis() {
        let params = SourceParams {
            source_type: SourceType::Redis,
            url: "redis://localhost:6379",
            api_key: Some("redis-pass"),
            collection: "my-idx",
        };

        let result = build_source_config(&params);
        assert!(result.is_ok());
        match result.unwrap() {
            SourceConfig::Redis(cfg) => {
                assert_eq!(cfg.url, "redis://localhost:6379");
                assert_eq!(cfg.index, "my-idx");
                assert_eq!(cfg.password, Some("redis-pass".to_string()));
            }
            _ => panic!("Expected Redis config"),
        }
    }

    #[test]
    fn test_build_source_config_missing_api_key_rejected() {
        // Supabase requires an API key — None must be rejected.
        let params = SourceParams {
            source_type: SourceType::Supabase,
            url: "https://xyz.supabase.co",
            api_key: None,
            collection: "docs",
        };
        assert!(build_source_config(&params).is_err());
    }

    #[test]
    fn test_build_source_config_empty_api_key_rejected() {
        // An empty string is not a valid API key.
        let params = SourceParams {
            source_type: SourceType::Supabase,
            url: "https://xyz.supabase.co",
            api_key: Some(""),
            collection: "docs",
        };
        assert!(build_source_config(&params).is_err());
    }

    #[test]
    fn test_pinecone_missing_api_key_rejected() {
        let params = SourceParams {
            source_type: SourceType::Pinecone,
            url: "https://index.pinecone.io",
            api_key: None,
            collection: "my-index",
        };
        assert!(build_source_config(&params).is_err());
    }

    #[test]
    fn test_mongodb_missing_api_key_rejected() {
        let params = SourceParams {
            source_type: SourceType::MongoDB,
            url: "https://data.mongodb-api.com/v1",
            api_key: None,
            collection: "embeddings",
        };
        assert!(build_source_config(&params).is_err());
    }
}
