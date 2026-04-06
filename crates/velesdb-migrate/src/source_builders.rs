//! Individual source config builder functions.
//!
//! Each function constructs a `SourceConfig` variant from `SourceParams`.
//! Extracted from `source_config_builder.rs` to keep that module focused
//! on the public API (`build_source_config`, `parse_source_type`, `fetch_schema`).

use crate::config::SourceConfig;
use crate::error::Result;

use super::source_config_builder::{require_api_key, SourceParams};

pub(crate) fn build_supabase(params: &SourceParams<'_>) -> Result<SourceConfig> {
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

pub(crate) fn build_qdrant(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Qdrant(crate::config::QdrantConfig {
        url: params.url.to_string(),
        collection: params.collection.to_string(),
        api_key: params.api_key.map(String::from),
        payload_fields: vec![],
    })
}

#[allow(deprecated)]
pub(crate) fn build_pinecone(params: &SourceParams<'_>) -> Result<SourceConfig> {
    let api_key = require_api_key(params, "Pinecone")?;
    Ok(SourceConfig::Pinecone(crate::config::PineconeConfig {
        api_key,
        environment: String::new(),
        index: params.collection.to_string(),
        namespace: None,
        base_url: None,
    }))
}

pub(crate) fn build_weaviate(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Weaviate(crate::config::WeaviateConfig {
        url: params.url.to_string(),
        class_name: params.collection.to_string(),
        api_key: params.api_key.map(String::from),
        properties: vec![],
    })
}

pub(crate) fn build_milvus(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Milvus(crate::config::MilvusConfig {
        url: params.url.to_string(),
        collection: params.collection.to_string(),
        username: None,
        password: None,
    })
}

pub(crate) fn build_chromadb(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::ChromaDB(crate::config::ChromaDBConfig {
        url: params.url.to_string(),
        collection: params.collection.to_string(),
        tenant: None,
        database: None,
    })
}

pub(crate) fn build_pgvector(params: &SourceParams<'_>) -> Result<SourceConfig> {
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
        Err(crate::error::Error::Config(
            "pgvector requires --features postgres".to_string(),
        ))
    }
}

pub(crate) fn build_json_file(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::JsonFile(crate::connectors::json_file::JsonFileConfig {
        path: std::path::PathBuf::from(params.url),
        array_path: String::new(),
        id_field: "id".to_string(),
        vector_field: "vector".to_string(),
        payload_fields: vec![],
    })
}

pub(crate) fn build_csv_file(params: &SourceParams<'_>) -> SourceConfig {
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

pub(crate) fn build_mongodb(params: &SourceParams<'_>) -> Result<SourceConfig> {
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
            data_source: "mongodb-atlas".to_string(),
        },
    ))
}

pub(crate) fn build_elasticsearch(params: &SourceParams<'_>) -> SourceConfig {
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

pub(crate) fn build_redis(params: &SourceParams<'_>) -> SourceConfig {
    SourceConfig::Redis(crate::config::RedisConfig {
        url: params.url.to_string(),
        password: params.api_key.map(String::from),
        index: params.collection.to_string(),
        vector_field: "embedding".to_string(),
        key_prefix: "doc:".to_string(),
        payload_fields: vec![],
        filter: None,
    })
}
