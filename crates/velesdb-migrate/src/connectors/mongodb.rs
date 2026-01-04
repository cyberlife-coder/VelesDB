//! MongoDB Atlas Vector Search connector.
//!
//! This module provides a connector for importing vectors from MongoDB Atlas
//! with Vector Search enabled. Uses the MongoDB Data API (REST) for compatibility.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::connectors::common::{
    create_http_client, handle_http_error, json_type_name, validate_url,
};
use crate::connectors::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::error::{Error, Result};

/// Configuration for MongoDB Atlas Vector Search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoDBConfig {
    /// MongoDB Data API endpoint URL.
    /// Format: https://data.mongodb-api.com/app/<app-id>/endpoint/data/v1
    pub data_api_url: String,
    /// MongoDB Data API key.
    pub api_key: String,
    /// Database name.
    pub database: String,
    /// Collection name containing vectors.
    pub collection: String,
    /// Field name containing the vector embedding.
    #[serde(default = "default_vector_field")]
    pub vector_field: String,
    /// Field name for document ID (default: "_id").
    #[serde(default = "default_id_field")]
    pub id_field: String,
    /// Fields to include in payload (empty = all except _id and vector).
    #[serde(default)]
    pub payload_fields: Vec<String>,
    /// Optional filter query (MongoDB query syntax as JSON).
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

fn default_vector_field() -> String {
    "embedding".to_string()
}

fn default_id_field() -> String {
    "_id".to_string()
}

/// Request body for MongoDB Data API find operation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FindRequest {
    data_source: String,
    database: String,
    collection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    projection: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
}

/// Response from MongoDB Data API find operation.
#[derive(Debug, Deserialize)]
struct FindResponse {
    documents: Vec<serde_json::Value>,
}

/// Request for aggregate operation (used for counting).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AggregateRequest {
    data_source: String,
    database: String,
    collection: String,
    pipeline: Vec<serde_json::Value>,
}

/// Response from aggregate operation.
#[derive(Debug, Deserialize)]
struct AggregateResponse {
    documents: Vec<serde_json::Value>,
}

/// MongoDB Atlas Vector Search connector.
pub struct MongoDBConnector {
    config: MongoDBConfig,
    client: Client,
    schema: Option<SourceSchema>,
    data_source: String,
}

impl MongoDBConnector {
    /// Creates a new MongoDB connector with configured HTTP client.
    pub fn new(config: MongoDBConfig) -> Self {
        Self {
            config,
            client: create_http_client(),
            schema: None,
            data_source: "mongodb-atlas".to_string(),
        }
    }

    /// Builds the API URL for a specific action.
    fn build_url(&self, action: &str) -> String {
        format!(
            "{}/action/{}",
            self.config.data_api_url.trim_end_matches('/'),
            action
        )
    }

    /// Makes a POST request to the MongoDB Data API.
    async fn api_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        action: &str,
        body: &T,
    ) -> Result<R> {
        let url = self.build_url(action);

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("api-key", &self.config.api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| Error::SourceConnection(format!("MongoDB API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(handle_http_error(status.as_u16(), &body, "MongoDB"));
        }

        response
            .json()
            .await
            .map_err(|e| Error::Extraction(format!("Failed to parse MongoDB response: {}", e)))
    }

    /// Gets the total count of documents.
    async fn get_count(&self) -> Result<u64> {
        let mut pipeline = vec![];

        if let Some(filter) = &self.config.filter {
            pipeline.push(serde_json::json!({ "$match": filter }));
        }

        pipeline.push(serde_json::json!({ "$count": "total" }));

        let request = AggregateRequest {
            data_source: self.data_source.clone(),
            database: self.config.database.clone(),
            collection: self.config.collection.clone(),
            pipeline,
        };

        let response: AggregateResponse = self.api_request("aggregate", &request).await?;

        Ok(response
            .documents
            .first()
            .and_then(|doc| doc.get("total"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0))
    }

    /// Parses a vector from a MongoDB document.
    fn parse_vector(&self, doc: &serde_json::Value) -> Result<Vec<f32>> {
        let vector_value = doc.get(&self.config.vector_field).ok_or_else(|| {
            Error::Extraction(format!(
                "Vector field '{}' not found in document",
                self.config.vector_field
            ))
        })?;

        match vector_value {
            serde_json::Value::Array(arr) => arr
                .iter()
                .map(|v| {
                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                        Error::Extraction("Vector element is not a number".to_string())
                    })
                })
                .collect(),
            _ => Err(Error::Extraction(format!(
                "Vector field '{}' is not an array",
                self.config.vector_field
            ))),
        }
    }

    /// Extracts ID from a MongoDB document.
    fn extract_id(&self, doc: &serde_json::Value) -> String {
        doc.get(&self.config.id_field)
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(obj) => {
                    // Handle MongoDB ObjectId: {"$oid": "..."}
                    obj.get("$oid")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string())
                }
                _ => v.to_string(),
            })
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Extracts payload from a MongoDB document.
    fn extract_payload(&self, doc: &serde_json::Value) -> HashMap<String, serde_json::Value> {
        let mut payload = HashMap::new();

        if let serde_json::Value::Object(map) = doc {
            for (key, val) in map {
                // Skip _id and vector fields
                if key == &self.config.id_field || key == &self.config.vector_field {
                    continue;
                }
                // If payload_fields is specified, only include those
                if !self.config.payload_fields.is_empty()
                    && !self.config.payload_fields.contains(key)
                {
                    continue;
                }
                payload.insert(key.clone(), val.clone());
            }
        }

        payload
    }
}

#[async_trait]
impl SourceConnector for MongoDBConnector {
    fn source_type(&self) -> &'static str {
        "mongodb"
    }

    async fn connect(&mut self) -> Result<()> {
        // Validate URL before connecting
        validate_url(&self.config.data_api_url)?;

        // Fetch a sample document to detect schema
        let request = FindRequest {
            data_source: self.data_source.clone(),
            database: self.config.database.clone(),
            collection: self.config.collection.clone(),
            filter: self.config.filter.clone(),
            projection: None,
            skip: None,
            limit: Some(1),
        };

        let response: FindResponse = self.api_request("find", &request).await?;

        if response.documents.is_empty() {
            return Err(Error::Extraction(
                "No documents found in MongoDB collection".to_string(),
            ));
        }

        let sample = &response.documents[0];
        let vector = self.parse_vector(sample)?;
        let dimension = vector.len();

        // Get total count
        let total_count = self.get_count().await?;

        // Detect fields
        let mut fields = Vec::new();
        if let serde_json::Value::Object(map) = sample {
            for (key, val) in map {
                if key == &self.config.id_field || key == &self.config.vector_field {
                    continue;
                }
                fields.push(FieldInfo {
                    name: key.clone(),
                    field_type: json_type_name(val),
                    indexed: false,
                });
            }
        }

        self.schema = Some(SourceSchema {
            source_type: "mongodb".to_string(),
            collection: self.config.collection.clone(),
            dimension,
            total_count: Some(total_count),
            fields,
            vector_column: Some(self.config.vector_field.clone()),
            id_column: Some(self.config.id_field.clone()),
        });

        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        self.schema
            .clone()
            .ok_or_else(|| Error::SourceConnection("Not connected".to_string()))
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let skip = offset.and_then(|v| v.as_u64()).unwrap_or(0);

        let request = FindRequest {
            data_source: self.data_source.clone(),
            database: self.config.database.clone(),
            collection: self.config.collection.clone(),
            filter: self.config.filter.clone(),
            projection: None,
            skip: Some(skip),
            limit: Some(batch_size as u64),
        };

        let response: FindResponse = self.api_request("find", &request).await?;

        let mut points = Vec::with_capacity(response.documents.len());
        for doc in &response.documents {
            let id = self.extract_id(doc);
            let vector = self.parse_vector(doc)?;
            let payload = self.extract_payload(doc);

            points.push(ExtractedPoint {
                id,
                vector,
                payload,
            });
        }

        let fetched = points.len() as u64;
        let has_more = fetched == batch_size as u64;
        let next_offset = if has_more {
            Some(serde_json::json!(skip + fetched))
        } else {
            None
        };

        Ok(ExtractedBatch {
            points,
            next_offset,
            has_more,
        })
    }

    async fn close(&mut self) -> Result<()> {
        self.schema = None;
        Ok(())
    }
}

#[cfg(test)]
#[path = "mongodb_tests.rs"]
mod tests;
