//! Weaviate vector database connector.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

use super::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::config::WeaviateConfig;
use crate::error::{Error, Result};

/// Weaviate source connector.
pub struct WeaviateConnector {
    config: WeaviateConfig,
    client: reqwest::Client,
}

impl WeaviateConnector {
    /// Create a new Weaviate connector.
    #[must_use]
    pub fn new(config: WeaviateConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.config.url.trim_end_matches('/'), path);
        let mut req = self.client.request(method, &url);

        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        req.header("Content-Type", "application/json")
    }
}

#[derive(Debug, Deserialize)]
struct SchemaResponse {
    classes: Vec<ClassSchema>,
}

#[derive(Debug, Deserialize)]
struct ClassSchema {
    class: String,
    properties: Option<Vec<PropertySchema>>,
    #[serde(rename = "vectorIndexConfig")]
    #[allow(dead_code)] // Parsed from JSON, reserved for distance metric
    vector_index_config: Option<VectorIndexConfig>,
}

#[derive(Debug, Deserialize)]
struct PropertySchema {
    name: String,
    #[serde(rename = "dataType")]
    data_type: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct VectorIndexConfig {
    #[allow(dead_code)] // Parsed from JSON, reserved for auto-detecting metric
    distance: Option<String>,
}

#[derive(Debug, Serialize)]
struct GraphQLQuery {
    query: String,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    data: Option<GraphQLData>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLData {
    #[serde(rename = "Get")]
    get: Option<HashMap<String, Vec<WeaviateObject>>>,
    #[serde(rename = "Aggregate")]
    aggregate: Option<HashMap<String, Vec<AggregateResult>>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct WeaviateObject {
    #[serde(rename = "_additional")]
    additional: Option<AdditionalInfo>,
    #[serde(flatten)]
    properties: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
struct AdditionalInfo {
    id: Option<String>,
    vector: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
struct AggregateResult {
    meta: Option<MetaCount>,
}

#[derive(Debug, Deserialize)]
struct MetaCount {
    count: Option<u64>,
}

#[async_trait]
impl SourceConnector for WeaviateConnector {
    fn source_type(&self) -> &'static str {
        "weaviate"
    }

    async fn connect(&mut self) -> Result<()> {
        info!("Connecting to Weaviate at {}", self.config.url);

        let resp = self
            .request(reqwest::Method::GET, "/v1/.well-known/ready")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::SourceConnection("Weaviate is not ready".to_string()));
        }

        info!("Connected to Weaviate, class: {}", self.config.class_name);
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        // Get class schema
        let resp = self
            .request(reqwest::Method::GET, "/v1/schema")
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "Failed to get Weaviate schema: {status} - {body}"
            )));
        }

        let schema: SchemaResponse = resp.json().await?;

        let class = schema
            .classes
            .iter()
            .find(|c| c.class == self.config.class_name)
            .ok_or_else(|| {
                Error::Extraction(format!("Class '{}' not found", self.config.class_name))
            })?;

        let fields: Vec<FieldInfo> = class
            .properties
            .as_ref()
            .map(|props| {
                props
                    .iter()
                    .map(|p| FieldInfo {
                        name: p.name.clone(),
                        field_type: p.data_type.first().cloned().unwrap_or_default(),
                        indexed: true,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Get count via GraphQL
        let count_query = format!(
            r"{{ Aggregate {{ {} {{ meta {{ count }} }} }} }}",
            self.config.class_name
        );

        let resp = self
            .request(reqwest::Method::POST, "/v1/graphql")
            .json(&GraphQLQuery { query: count_query })
            .send()
            .await?;

        let gql_resp: GraphQLResponse = resp.json().await?;

        let total_count = gql_resp
            .data
            .and_then(|d| d.aggregate)
            .and_then(|mut a| a.remove(&self.config.class_name))
            .and_then(|v| v.into_iter().next())
            .and_then(|r| r.meta)
            .and_then(|m| m.count);

        // Fetch one object to determine vector dimension
        let mut dimension = 0;
        let peek_query = format!(
            r"{{ Get {{ {} (limit: 1) {{ _additional {{ vector }} }} }} }}",
            self.config.class_name
        );

        let peek_resp = self
            .request(reqwest::Method::POST, "/v1/graphql")
            .json(&GraphQLQuery { query: peek_query })
            .send()
            .await?;

        if peek_resp.status().is_success() {
            let peek_data: serde_json::Value = peek_resp.json().await?;
            if let Some(vector) = peek_data
                .get("data")
                .and_then(|d| d.get("Get"))
                .and_then(|g| g.get(&self.config.class_name))
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|obj| obj.get("_additional"))
                .and_then(|add| add.get("vector"))
                .and_then(|v| v.as_array())
            {
                dimension = vector.len();
            }
        }

        info!(
            "Weaviate class '{}': {}D vectors, {:?} objects, {} properties",
            self.config.class_name,
            dimension,
            total_count,
            fields.len()
        );

        Ok(SourceSchema {
            source_type: "weaviate".to_string(),
            collection: self.config.class_name.clone(),
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
        let cursor = offset.and_then(|v| v.as_str().map(String::from));

        let properties = if self.config.properties.is_empty() {
            "_additional { id vector }".to_string()
        } else {
            format!(
                "{} _additional {{ id vector }}",
                self.config.properties.join(" ")
            )
        };

        let after_clause = cursor
            .as_ref()
            .map(|c| format!(r#", after: "{c}""#))
            .unwrap_or_default();

        let query = format!(
            r"{{ Get {{ {class}(limit: {limit}{after}) {{ {props} }} }} }}",
            class = self.config.class_name,
            limit = batch_size,
            after = after_clause,
            props = properties
        );

        debug!("Extracting batch from Weaviate");

        let resp = self
            .request(reqwest::Method::POST, "/v1/graphql")
            .json(&GraphQLQuery { query })
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "Weaviate GraphQL failed: {status} - {body}"
            )));
        }

        let gql_resp: GraphQLResponse = resp.json().await?;

        if let Some(errors) = gql_resp.errors {
            let msg = errors
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::Extraction(format!("GraphQL errors: {msg}")));
        }

        let objects = gql_resp
            .data
            .and_then(|d| d.get)
            .and_then(|mut g| g.remove(&self.config.class_name))
            .unwrap_or_default();

        let mut points = Vec::with_capacity(objects.len());
        let mut last_id: Option<String> = None;

        for obj in objects {
            let additional = obj.additional.unwrap_or_default();
            let id = additional.id.unwrap_or_default();
            let vector = additional.vector.unwrap_or_default();

            last_id = Some(id.clone());

            points.push(ExtractedPoint {
                id,
                vector,
                payload: obj.properties,
            });
        }

        let has_more = points.len() == batch_size;
        let next_offset = if has_more {
            last_id.map(serde_json::Value::String)
        } else {
            None
        };

        debug!("Extracted {} objects from Weaviate", points.len());

        Ok(ExtractedBatch {
            points,
            next_offset,
            has_more,
        })
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing Weaviate connection");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weaviate_connector_new() {
        let config = WeaviateConfig {
            url: "http://localhost:8080".to_string(),
            class_name: "Document".to_string(),
            api_key: None,
            properties: vec!["title".to_string()],
        };

        let connector = WeaviateConnector::new(config);
        assert_eq!(connector.source_type(), "weaviate");
    }

    #[test]
    fn test_graphql_query_serialization() {
        let query = GraphQLQuery {
            query: "{ Get { Document { title } } }".to_string(),
        };

        let json = serde_json::to_string(&query).unwrap();
        assert!(json.contains("Get"));
    }
}
