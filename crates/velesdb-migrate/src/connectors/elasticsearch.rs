//! Elasticsearch/OpenSearch connector for vector search.
//!
//! This module provides a connector for importing vectors from Elasticsearch
//! with dense_vector fields or OpenSearch with k-NN plugin.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::connectors::common::{create_http_client, handle_http_error, json_type_name};
use crate::connectors::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::error::{Error, Result};

/// Configuration for Elasticsearch/OpenSearch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElasticsearchConfig {
    /// Elasticsearch/OpenSearch URL (e.g., http://localhost:9200).
    pub url: String,
    /// Index name containing vectors.
    pub index: String,
    /// Field name containing the vector embedding.
    #[serde(default = "default_vector_field")]
    pub vector_field: String,
    /// Field name for document ID (default: "_id").
    #[serde(default = "default_id_field")]
    pub id_field: String,
    /// Fields to include in payload (empty = all except _id and vector).
    #[serde(default)]
    pub payload_fields: Vec<String>,
    /// Optional username for Basic auth.
    pub username: Option<String>,
    /// Optional password for Basic auth.
    pub password: Option<String>,
    /// Optional API key for authentication.
    pub api_key: Option<String>,
    /// Optional query filter (Elasticsearch DSL).
    #[serde(default)]
    pub query: Option<serde_json::Value>,
}

fn default_vector_field() -> String {
    "embedding".to_string()
}

fn default_id_field() -> String {
    "_id".to_string()
}

/// Search request body for Elasticsearch.
#[derive(Debug, Serialize)]
struct SearchRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_after: Option<Vec<serde_json::Value>>,
}

/// Search response from Elasticsearch.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: HitsContainer,
}

#[derive(Debug, Deserialize)]
struct HitsContainer {
    #[allow(dead_code)]
    total: TotalHits,
    hits: Vec<Hit>,
}

#[derive(Debug, Deserialize)]
struct TotalHits {
    #[allow(dead_code)]
    value: u64,
}

#[derive(Debug, Deserialize)]
struct Hit {
    #[serde(rename = "_id")]
    id: String,
    #[serde(rename = "_source")]
    source: serde_json::Value,
    #[serde(default)]
    sort: Option<Vec<serde_json::Value>>,
}

/// Elasticsearch/OpenSearch connector.
pub struct ElasticsearchConnector {
    config: ElasticsearchConfig,
    client: Client,
    schema: Option<SourceSchema>,
}

impl ElasticsearchConnector {
    /// Creates a new Elasticsearch connector with configured HTTP client.
    pub fn new(config: ElasticsearchConfig) -> Self {
        Self {
            config,
            client: create_http_client(),
            schema: None,
        }
    }

    /// Builds the search URL for the index.
    fn build_search_url(&self) -> String {
        format!(
            "{}{}/_search",
            self.config.url.trim_end_matches('/'),
            if self.config.index.starts_with('/') {
                self.config.index.clone()
            } else {
                format!("/{}", self.config.index)
            }
        )
    }

    /// Builds the count URL for the index.
    fn build_count_url(&self) -> String {
        format!(
            "{}{}/_count",
            self.config.url.trim_end_matches('/'),
            if self.config.index.starts_with('/') {
                self.config.index.clone()
            } else {
                format!("/{}", self.config.index)
            }
        )
    }

    /// Makes an authenticated request.
    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.post(url);
        req = req.header("Content-Type", "application/json");

        // Apply authentication
        if let Some(api_key) = &self.config.api_key {
            req = req.header("Authorization", format!("ApiKey {}", api_key));
        } else if let (Some(user), Some(pass)) = (&self.config.username, &self.config.password) {
            req = req.basic_auth(user, Some(pass));
        }

        req
    }

    /// Gets the total count of documents.
    async fn get_count(&self) -> Result<u64> {
        let url = self.build_count_url();
        let body = if let Some(query) = &self.config.query {
            serde_json::json!({ "query": query })
        } else {
            serde_json::json!({ "query": { "match_all": {} } })
        };

        let response = self
            .build_request(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::SourceConnection(format!("Elasticsearch request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(Error::SourceConnection(format!(
                "Elasticsearch error {}: {}",
                status, body
            )));
        }

        #[derive(Deserialize)]
        struct CountResponse {
            count: u64,
        }

        let count_resp: CountResponse = response
            .json()
            .await
            .map_err(|e| Error::Extraction(format!("Failed to parse count response: {}", e)))?;

        Ok(count_resp.count)
    }

    /// Parses a vector from an Elasticsearch document.
    fn parse_vector(&self, source: &serde_json::Value) -> Result<Vec<f32>> {
        let vector_value = source.get(&self.config.vector_field).ok_or_else(|| {
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

    /// Extracts payload from an Elasticsearch document.
    fn extract_payload(&self, source: &serde_json::Value) -> HashMap<String, serde_json::Value> {
        let mut payload = HashMap::new();

        if let serde_json::Value::Object(map) = source {
            for (key, val) in map {
                // Skip vector field
                if key == &self.config.vector_field {
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
impl SourceConnector for ElasticsearchConnector {
    fn source_type(&self) -> &'static str {
        "elasticsearch"
    }

    async fn connect(&mut self) -> Result<()> {
        // Fetch a sample document to detect schema
        let url = self.build_search_url();
        let body = SearchRequest {
            query: self
                .config
                .query
                .clone()
                .or(Some(serde_json::json!({ "match_all": {} }))),
            size: Some(1),
            from: None,
            sort: Some(vec![serde_json::json!({ "_id": "asc" })]),
            search_after: None,
        };

        let response = self
            .build_request(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::SourceConnection(format!("Elasticsearch request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(handle_http_error(status.as_u16(), &body, "Elasticsearch"));
        }

        let search_resp: SearchResponse = response
            .json()
            .await
            .map_err(|e| Error::Extraction(format!("Failed to parse search response: {}", e)))?;

        if search_resp.hits.hits.is_empty() {
            return Err(Error::Extraction(
                "No documents found in Elasticsearch index".to_string(),
            ));
        }

        let sample = &search_resp.hits.hits[0];
        let vector = self.parse_vector(&sample.source)?;
        let dimension = vector.len();

        // Get total count
        let total_count = self.get_count().await?;

        // Detect fields
        let mut fields = Vec::new();
        if let serde_json::Value::Object(map) = &sample.source {
            for (key, val) in map {
                if key == &self.config.vector_field {
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
            source_type: "elasticsearch".to_string(),
            collection: self.config.index.clone(),
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
        let url = self.build_search_url();

        // Use search_after for efficient pagination
        let search_after = offset.and_then(|v| v.as_array().cloned());

        let body = SearchRequest {
            query: self
                .config
                .query
                .clone()
                .or(Some(serde_json::json!({ "match_all": {} }))),
            size: Some(batch_size),
            from: None,
            sort: Some(vec![serde_json::json!({ "_id": "asc" })]),
            search_after,
        };

        let response = self
            .build_request(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::SourceConnection(format!("Elasticsearch request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            if status.as_u16() == 429 {
                return Err(Error::RateLimit(60));
            }
            return Err(Error::SourceConnection(format!(
                "Elasticsearch error {}: {}",
                status, body
            )));
        }

        let search_resp: SearchResponse = response
            .json()
            .await
            .map_err(|e| Error::Extraction(format!("Failed to parse search response: {}", e)))?;

        let mut points = Vec::with_capacity(search_resp.hits.hits.len());
        let mut last_sort: Option<Vec<serde_json::Value>> = None;

        for hit in &search_resp.hits.hits {
            let id = hit.id.clone();
            let vector = self.parse_vector(&hit.source)?;
            let payload = self.extract_payload(&hit.source);

            points.push(ExtractedPoint {
                id,
                vector,
                payload,
            });
            last_sort = hit.sort.clone();
        }

        let has_more = points.len() == batch_size;
        let next_offset = if has_more {
            last_sort.map(|s| serde_json::json!(s))
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
#[path = "elasticsearch_tests.rs"]
mod tests;
