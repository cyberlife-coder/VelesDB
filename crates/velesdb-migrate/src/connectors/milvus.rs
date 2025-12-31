//! Milvus vector database connector.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

use super::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::config::MilvusConfig;
use crate::error::{Error, Result};

/// Milvus source connector.
pub struct MilvusConnector {
    config: MilvusConfig,
    client: reqwest::Client,
}

impl MilvusConnector {
    /// Create a new Milvus connector.
    #[must_use]
    pub fn new(config: MilvusConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Build request with optional auth.
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!(
            "{}/v2/vectordb{}",
            self.config.url.trim_end_matches('/'),
            path
        );
        let mut req = self.client.request(method, &url);

        if let (Some(user), Some(pass)) = (&self.config.username, &self.config.password) {
            req = req.basic_auth(user, Some(pass));
        }

        req.header("Content-Type", "application/json")
    }
}

#[derive(Debug, Deserialize)]
struct MilvusResponse<T> {
    code: i32,
    data: Option<T>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields used for deserialization
struct CollectionInfo {
    #[serde(rename = "collectionName")]
    collection_name: String,
    #[serde(rename = "shardsNum")]
    shards_num: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct CollectionSchema {
    fields: Vec<FieldSchema>,
}

#[derive(Debug, Deserialize)]
struct FieldSchema {
    name: String,
    #[serde(rename = "type")]
    field_type: String,
    #[serde(rename = "isPrimaryKey")]
    is_primary_key: Option<bool>,
    params: Option<FieldParams>,
}

#[derive(Debug, Deserialize)]
struct FieldParams {
    dim: Option<usize>,
}

#[derive(Debug, Serialize)]
struct QueryRequest {
    #[serde(rename = "collectionName")]
    collection_name: String,
    filter: String,
    limit: usize,
    offset: usize,
    #[serde(rename = "outputFields")]
    output_fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Reserved for future query endpoint
struct QueryResponse {
    data: Vec<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct StatsResponse {
    #[serde(rename = "rowCount")]
    row_count: u64,
}

#[async_trait]
impl SourceConnector for MilvusConnector {
    fn source_type(&self) -> &'static str {
        "milvus"
    }

    async fn connect(&mut self) -> Result<()> {
        info!("Connecting to Milvus at {}", self.config.url);

        let resp = self
            .request(reqwest::Method::GET, "/collections/has")
            .query(&[("collectionName", &self.config.collection)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::SourceConnection(format!(
                "Milvus connection failed: {status} - {body}"
            )));
        }

        let result: MilvusResponse<bool> = resp.json().await?;

        if result.code != 0 {
            return Err(Error::SourceConnection(
                result
                    .message
                    .unwrap_or_else(|| "Unknown error".to_string()),
            ));
        }

        if result.data != Some(true) {
            return Err(Error::SourceConnection(format!(
                "Collection '{}' does not exist",
                self.config.collection
            )));
        }

        info!("Connected to Milvus collection: {}", self.config.collection);
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        // Get collection schema
        let resp = self
            .request(reqwest::Method::GET, "/collections/describe")
            .query(&[("collectionName", &self.config.collection)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "Failed to get Milvus schema: {status} - {body}"
            )));
        }

        let result: MilvusResponse<CollectionSchema> = resp.json().await?;

        let schema = result
            .data
            .ok_or_else(|| Error::Extraction("No schema data returned".to_string()))?;

        let (vector_field, dimension) = Self::find_vector_field(&schema)?;

        let fields: Vec<FieldInfo> = schema
            .fields
            .iter()
            .filter(|f| f.name != vector_field)
            .map(|f| FieldInfo {
                name: f.name.clone(),
                field_type: f.field_type.clone(),
                indexed: f.is_primary_key.unwrap_or(false),
            })
            .collect();

        // Get stats for count
        let resp = self
            .request(reqwest::Method::GET, "/collections/stats")
            .query(&[("collectionName", &self.config.collection)])
            .send()
            .await?;

        let total_count = if resp.status().is_success() {
            let stats: MilvusResponse<StatsResponse> = resp.json().await?;
            stats.data.map(|s| s.row_count)
        } else {
            None
        };

        info!(
            "Milvus collection '{}': {}D vectors, {:?} rows",
            self.config.collection, dimension, total_count
        );

        Ok(SourceSchema {
            source_type: "milvus".to_string(),
            collection: self.config.collection.clone(),
            dimension,
            total_count,
            fields,
            ..Default::default()
        })
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let current_offset = offset
            .as_ref()
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;

        let schema = self.get_schema().await?;
        let (vector_field, _) = Self::find_vector_field_from_schema(&schema)?;

        let mut output_fields: Vec<String> = schema.fields.iter().map(|f| f.name.clone()).collect();
        output_fields.push(vector_field.clone());

        let query = QueryRequest {
            collection_name: self.config.collection.clone(),
            filter: String::new(),
            limit: batch_size,
            offset: current_offset,
            output_fields,
        };

        debug!("Extracting batch from Milvus, offset={}", current_offset);

        let resp = self
            .request(reqwest::Method::POST, "/entities/query")
            .json(&query)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "Milvus query failed: {status} - {body}"
            )));
        }

        let result: MilvusResponse<Vec<HashMap<String, serde_json::Value>>> = resp.json().await?;

        let rows = result.data.unwrap_or_default();
        let mut points = Vec::with_capacity(rows.len());

        for mut row in rows {
            let id = row
                .remove("id")
                .and_then(|v| match v {
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::String(s) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let vector = row
                .remove(&vector_field)
                .and_then(|v| {
                    if let serde_json::Value::Array(arr) = v {
                        arr.into_iter()
                            .filter_map(|x| x.as_f64().map(|f| f as f32))
                            .collect::<Vec<_>>()
                            .into()
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            points.push(ExtractedPoint {
                id,
                vector,
                payload: row,
            });
        }

        let has_more = points.len() == batch_size;
        let next_offset = if has_more {
            Some(serde_json::json!(current_offset + points.len()))
        } else {
            None
        };

        debug!("Extracted {} rows from Milvus", points.len());

        Ok(ExtractedBatch {
            points,
            next_offset,
            has_more,
        })
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing Milvus connection");
        Ok(())
    }
}

impl MilvusConnector {
    fn find_vector_field(schema: &CollectionSchema) -> Result<(String, usize)> {
        for field in &schema.fields {
            if field.field_type.contains("Vector") || field.field_type.contains("FLOAT_VECTOR") {
                let dim = field.params.as_ref().and_then(|p| p.dim).unwrap_or(0);
                return Ok((field.name.clone(), dim));
            }
        }
        Err(Error::SchemaMismatch("No vector field found".to_string()))
    }

    fn find_vector_field_from_schema(schema: &SourceSchema) -> Result<(String, usize)> {
        // Return default vector field name and dimension from schema
        Ok(("vector".to_string(), schema.dimension))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_milvus_connector_new() {
        let config = MilvusConfig {
            url: "http://localhost:19530".to_string(),
            collection: "test".to_string(),
            username: None,
            password: None,
        };

        let connector = MilvusConnector::new(config);
        assert_eq!(connector.source_type(), "milvus");
    }

    #[test]
    fn test_query_request_serialization() {
        let req = QueryRequest {
            collection_name: "test".to_string(),
            filter: "".to_string(),
            limit: 100,
            offset: 0,
            output_fields: vec!["id".to_string(), "vector".to_string()],
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"collectionName\":\"test\""));
        assert!(json.contains("\"limit\":100"));
    }
}
