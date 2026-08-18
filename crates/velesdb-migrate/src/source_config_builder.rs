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
    use super::source_builders::*;
    match params.source_type {
        SourceType::Supabase => build_supabase(params),
        SourceType::Qdrant => Ok(build_qdrant(params)),
        SourceType::Pinecone => build_pinecone(params),
        SourceType::Weaviate => Ok(build_weaviate(params)),
        SourceType::Milvus => Ok(build_milvus(params)),
        SourceType::ChromaDB => Ok(build_chromadb(params)),
        SourceType::JsonFile => Ok(build_json_file(params)),
        SourceType::CsvFile => Ok(build_csv_file(params)),
        SourceType::Elasticsearch => Ok(build_elasticsearch(params)),
        SourceType::Redis => Ok(build_redis(params)),
    }
}

/// Extracts and validates a required API key from source parameters.
///
/// # Errors
///
/// Returns `Error::Config` if the API key is `None` or empty.
pub(crate) fn require_api_key(params: &SourceParams<'_>, source_name: &str) -> Result<String> {
    params
        .api_key
        .filter(|k| !k.is_empty())
        .map(String::from)
        .ok_or_else(|| Error::Config(format!("{source_name} requires an API key (--api-key)")))
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
        "json_file" | "json" => Ok(SourceType::JsonFile),
        "csv_file" | "csv" => Ok(SourceType::CsvFile),
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
#[path = "source_config_builder_tests.rs"]
mod tests;
