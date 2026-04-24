//! Source type definitions for the migration wizard.
//!
//! Extracted from `wizard/mod.rs` to keep module size under 500 NLOC.

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
    /// JSON file import.
    JsonFile,
    /// CSV file import.
    CsvFile,
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
            Self::JsonFile,
            Self::CsvFile,
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
            Self::JsonFile => "JSON File (local import)",
            Self::CsvFile => "CSV File (local import)",
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
            Self::JsonFile => "json_file",
            Self::CsvFile => "csv_file",
            Self::Elasticsearch => "elasticsearch",
            Self::Redis => "redis",
        }
    }

    /// Whether this source requires an API key.
    pub fn requires_api_key(&self) -> bool {
        matches!(self, Self::Supabase | Self::Pinecone)
    }

    /// Whether API key is optional.
    pub fn optional_api_key(&self) -> bool {
        matches!(
            self,
            Self::Qdrant | Self::Weaviate | Self::Milvus | Self::Elasticsearch | Self::Redis
        )
    }
}
