//! Redis Vector Search connector.
//!
//! This module provides a connector for importing vectors from Redis Stack
//! with RediSearch module (FT.SEARCH). Supports both Redis Cloud and self-hosted.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::connectors::common::{create_http_client, handle_http_error};
use crate::connectors::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::error::{Error, Result};

/// Configuration for Redis Vector Search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// Redis URL (e.g., redis://localhost:6379 or rediss://... for TLS).
    pub url: String,
    /// Redis password (optional).
    #[serde(default)]
    pub password: Option<String>,
    /// Index name created with FT.CREATE.
    pub index: String,
    /// Field name containing the vector embedding.
    #[serde(default = "default_vector_field")]
    pub vector_field: String,
    /// Prefix for document keys (e.g., "doc:").
    #[serde(default = "default_key_prefix")]
    pub key_prefix: String,
    /// Fields to include in payload (empty = all).
    #[serde(default)]
    pub payload_fields: Vec<String>,
    /// Optional filter query (RediSearch syntax).
    #[serde(default)]
    pub filter: Option<String>,
}

fn default_vector_field() -> String {
    "embedding".to_string()
}

fn default_key_prefix() -> String {
    "doc:".to_string()
}

/// Redis REST API response for FT.SEARCH.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
    #[serde(default)]
    #[allow(dead_code)]
    total: u64,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    id: String,
    #[serde(default)]
    extra_attributes: HashMap<String, serde_json::Value>,
}

/// Redis REST API response for FT.INFO.
#[derive(Debug, Deserialize)]
struct IndexInfo {
    num_docs: u64,
    #[serde(default)]
    attributes: Vec<AttributeInfo>,
}

#[derive(Debug, Deserialize)]
struct AttributeInfo {
    identifier: String,
    #[serde(rename = "type")]
    attr_type: String,
}

/// Redis Vector Search connector.
pub struct RedisConnector {
    config: RedisConfig,
    client: Client,
    schema: Option<SourceSchema>,
    api_url: String,
}

impl RedisConnector {
    /// Creates a new Redis connector with configured HTTP client.
    pub fn new(config: RedisConfig) -> Self {
        let api_url = Self::build_api_url(&config.url);
        Self {
            config,
            client: create_http_client(),
            schema: None,
            api_url,
        }
    }

    /// Builds the REST API URL from Redis URL.
    fn build_api_url(redis_url: &str) -> String {
        // Convert redis:// to http:// for REST API
        let url = redis_url
            .replace("redis://", "http://")
            .replace("rediss://", "https://");
        url.trim_end_matches('/').to_string()
    }

    /// Executes a Redis command via REST API.
    async fn execute_command<T: for<'de> Deserialize<'de>>(
        &self,
        command: &str,
        args: &[&str],
    ) -> Result<T> {
        let url = format!("{}/{}", self.api_url, command);

        let mut request = self.client.post(&url);

        if let Some(password) = &self.config.password {
            request = request.header("Authorization", format!("Bearer {}", password));
        }

        let body = serde_json::json!({ "args": args });

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::SourceConnection(format!("Redis request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(handle_http_error(status.as_u16(), &body, "Redis"));
        }

        response
            .json()
            .await
            .map_err(|e| Error::Extraction(format!("Failed to parse Redis response: {}", e)))
    }

    /// Gets index information.
    async fn get_index_info(&self) -> Result<IndexInfo> {
        self.execute_command("FT.INFO", &[&self.config.index]).await
    }

    /// Searches the index.
    async fn search(&self, query: &str, offset: u64, limit: u64) -> Result<SearchResponse> {
        let offset_str = offset.to_string();
        let limit_str = limit.to_string();

        let mut args = vec![
            self.config.index.as_str(),
            query,
            "LIMIT",
            &offset_str,
            &limit_str,
            "RETURN",
            "10", // Return up to 10 fields
        ];

        // Add specific fields to return
        if !self.config.payload_fields.is_empty() {
            for field in &self.config.payload_fields {
                args.push(field.as_str());
            }
        }

        self.execute_command("FT.SEARCH", &args).await
    }

    /// Parses a vector from Redis document.
    pub fn parse_vector(&self, attrs: &HashMap<String, serde_json::Value>) -> Result<Vec<f32>> {
        let vector_value = attrs.get(&self.config.vector_field).ok_or_else(|| {
            Error::Extraction(format!(
                "Vector field '{}' not found in document",
                self.config.vector_field
            ))
        })?;

        // Redis stores vectors as byte arrays or JSON arrays
        match vector_value {
            serde_json::Value::Array(arr) => arr
                .iter()
                .map(|v| {
                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                        Error::Extraction("Vector element is not a number".to_string())
                    })
                })
                .collect(),
            serde_json::Value::String(s) => {
                // Parse comma-separated or space-separated values
                s.split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        s.trim()
                            .parse::<f32>()
                            .map_err(|_| Error::Extraction("Invalid vector element".to_string()))
                    })
                    .collect()
            }
            _ => Err(Error::Extraction(format!(
                "Vector field '{}' has unsupported format",
                self.config.vector_field
            ))),
        }
    }

    /// Extracts ID from Redis document key.
    pub fn extract_id(&self, key: &str) -> String {
        // Remove prefix if present
        key.strip_prefix(&self.config.key_prefix)
            .unwrap_or(key)
            .to_string()
    }

    /// Extracts payload from Redis document.
    pub fn extract_payload(
        &self,
        attrs: &HashMap<String, serde_json::Value>,
    ) -> HashMap<String, serde_json::Value> {
        let mut payload = HashMap::new();

        for (key, val) in attrs {
            // Skip vector field
            if key == &self.config.vector_field {
                continue;
            }
            // If payload_fields is specified, only include those
            if !self.config.payload_fields.is_empty() && !self.config.payload_fields.contains(key) {
                continue;
            }
            payload.insert(key.clone(), val.clone());
        }

        payload
    }
}

#[async_trait]
impl SourceConnector for RedisConnector {
    fn source_type(&self) -> &'static str {
        "redis"
    }

    async fn connect(&mut self) -> Result<()> {
        // Get index info to detect schema
        let info = self.get_index_info().await?;

        // Search for a sample document
        let query = self
            .config
            .filter
            .clone()
            .unwrap_or_else(|| "*".to_string());
        let sample = self.search(&query, 0, 1).await?;

        if sample.results.is_empty() {
            return Err(Error::Extraction(
                "No documents found in Redis index".to_string(),
            ));
        }

        let doc = &sample.results[0];
        let vector = self.parse_vector(&doc.extra_attributes)?;
        let dimension = vector.len();

        // Detect fields from index info
        let fields: Vec<FieldInfo> = info
            .attributes
            .iter()
            .filter(|a| a.identifier != self.config.vector_field)
            .map(|a| FieldInfo {
                name: a.identifier.clone(),
                field_type: a.attr_type.clone(),
                indexed: true,
            })
            .collect();

        self.schema = Some(SourceSchema {
            source_type: "redis".to_string(),
            collection: self.config.index.clone(),
            dimension,
            total_count: Some(info.num_docs),
            fields,
            vector_column: Some(self.config.vector_field.clone()),
            id_column: None,
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
        let offset_num = offset.and_then(|v| v.as_u64()).unwrap_or(0);

        let query = self
            .config
            .filter
            .clone()
            .unwrap_or_else(|| "*".to_string());

        #[allow(clippy::cast_possible_truncation)]
        let response = self.search(&query, offset_num, batch_size as u64).await?;

        let mut points = Vec::with_capacity(response.results.len());

        for doc in &response.results {
            let id = self.extract_id(&doc.id);
            let vector = self.parse_vector(&doc.extra_attributes)?;
            let payload = self.extract_payload(&doc.extra_attributes);

            points.push(ExtractedPoint {
                id,
                vector,
                payload,
            });
        }

        let has_more = points.len() == batch_size;
        let next_offset = if has_more {
            Some(serde_json::json!(offset_num + batch_size as u64))
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
#[path = "redis_tests.rs"]
mod tests;
