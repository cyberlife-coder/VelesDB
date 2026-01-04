//! Source connectors for different vector databases.

pub mod chromadb;
pub mod common;
pub mod csv_file;
pub mod elasticsearch;
pub mod json_file;
pub mod milvus;
pub mod mongodb;
pub mod pgvector;
pub mod pinecone;
pub mod qdrant;
pub mod redis;
pub mod weaviate;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::Result;

/// A point extracted from a source database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedPoint {
    /// Unique identifier (string-based for compatibility).
    pub id: String,
    /// Vector embedding.
    pub vector: Vec<f32>,
    /// Metadata/payload.
    pub payload: HashMap<String, serde_json::Value>,
}

/// A batch of extracted points.
#[derive(Debug, Clone)]
pub struct ExtractedBatch {
    /// Points in this batch.
    pub points: Vec<ExtractedPoint>,
    /// Offset/cursor for pagination.
    pub next_offset: Option<serde_json::Value>,
    /// Whether there are more batches.
    pub has_more: bool,
}

/// Schema information from the source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceSchema {
    /// Source type name.
    pub source_type: String,
    /// Collection/table name.
    pub collection: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Total number of vectors (if known).
    pub total_count: Option<u64>,
    /// Available payload/metadata fields.
    pub fields: Vec<FieldInfo>,
    /// Detected vector column name (for SQL-based sources).
    #[serde(default)]
    pub vector_column: Option<String>,
    /// Detected ID column name (for SQL-based sources).
    #[serde(default)]
    pub id_column: Option<String>,
}

/// Information about a payload field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    /// Field name.
    pub name: String,
    /// Field type (string representation).
    pub field_type: String,
    /// Whether the field is indexed.
    pub indexed: bool,
}

/// Trait for source database connectors.
///
/// Implement this trait to add support for a new vector database source.
#[async_trait]
pub trait SourceConnector: Send + Sync {
    /// Get the source type name.
    fn source_type(&self) -> &'static str;

    /// Connect to the source and validate configuration.
    async fn connect(&mut self) -> Result<()>;

    /// Get schema information from the source.
    async fn get_schema(&self) -> Result<SourceSchema>;

    /// Extract a batch of points starting from an optional offset.
    ///
    /// # Arguments
    ///
    /// * `offset` - Optional offset/cursor for pagination
    /// * `batch_size` - Maximum number of points to extract
    ///
    /// # Returns
    ///
    /// A batch of extracted points with pagination info.
    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch>;

    /// Close the connection and cleanup resources.
    async fn close(&mut self) -> Result<()>;
}

/// Create a source connector from configuration.
pub fn create_connector(config: &crate::config::SourceConfig) -> Result<Box<dyn SourceConnector>> {
    match config {
        crate::config::SourceConfig::Qdrant(cfg) => {
            Ok(Box::new(qdrant::QdrantConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::Pinecone(cfg) => {
            Ok(Box::new(pinecone::PineconeConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::Weaviate(cfg) => {
            Ok(Box::new(weaviate::WeaviateConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::Milvus(cfg) => {
            Ok(Box::new(milvus::MilvusConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::ChromaDB(cfg) => {
            Ok(Box::new(chromadb::ChromaDBConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::PgVector(cfg) => {
            Ok(Box::new(pgvector::PgVectorConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::Supabase(cfg) => {
            Ok(Box::new(pgvector::SupabaseConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::JsonFile(cfg) => {
            Ok(Box::new(json_file::JsonFileConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::CsvFile(cfg) => {
            Ok(Box::new(csv_file::CsvFileConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::MongoDB(cfg) => {
            Ok(Box::new(mongodb::MongoDBConnector::new(cfg.clone())))
        }
        crate::config::SourceConfig::Elasticsearch(cfg) => Ok(Box::new(
            elasticsearch::ElasticsearchConnector::new(cfg.clone()),
        )),
        crate::config::SourceConfig::Redis(cfg) => {
            Ok(Box::new(redis::RedisConnector::new(cfg.clone())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extracted_point_serialization() {
        let point = ExtractedPoint {
            id: "test-123".to_string(),
            vector: vec![0.1, 0.2, 0.3],
            payload: HashMap::from([
                ("title".to_string(), serde_json::json!("Test Document")),
                ("score".to_string(), serde_json::json!(0.95)),
            ]),
        };

        let json = serde_json::to_string(&point).unwrap();
        let parsed: ExtractedPoint = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "test-123");
        assert_eq!(parsed.vector.len(), 3);
    }

    #[test]
    fn test_source_schema() {
        let schema = SourceSchema {
            source_type: "qdrant".to_string(),
            collection: "documents".to_string(),
            dimension: 768,
            total_count: Some(10000),
            fields: vec![FieldInfo {
                name: "title".to_string(),
                field_type: "string".to_string(),
                indexed: true,
            }],
            ..Default::default()
        };

        assert_eq!(schema.dimension, 768);
        assert_eq!(schema.total_count, Some(10000));
    }
}
