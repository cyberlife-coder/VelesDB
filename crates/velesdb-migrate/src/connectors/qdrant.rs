//! Qdrant vector database connector.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

use super::{ExtractedBatch, ExtractedPoint, SourceConnector, SourceSchema};
use crate::config::QdrantConfig;
use crate::error::{Error, Result};

/// Qdrant source connector.
pub struct QdrantConnector {
    config: QdrantConfig,
    client: reqwest::Client,
}

impl QdrantConnector {
    /// Create a new Qdrant connector.
    #[must_use]
    pub fn new(config: QdrantConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Build request with optional auth.
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!(
            "{}/collections/{}{}",
            self.config.url.trim_end_matches('/'),
            self.config.collection,
            path
        );
        let mut req = self.client.request(method, &url);

        if let Some(ref key) = self.config.api_key {
            req = req.header("api-key", key);
        }

        req.header("Content-Type", "application/json")
    }
}

#[derive(Debug, Deserialize)]
struct QdrantCollectionInfo {
    result: QdrantCollectionResult,
}

#[derive(Debug, Deserialize)]
struct QdrantCollectionResult {
    vectors_count: Option<u64>,
    points_count: Option<u64>,
    config: QdrantCollectionConfig,
}

#[derive(Debug, Deserialize)]
struct QdrantCollectionConfig {
    params: QdrantParams,
}

#[derive(Debug, Deserialize)]
struct QdrantParams {
    vectors: QdrantVectorConfig,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QdrantVectorConfig {
    Single { size: usize },
    Named(HashMap<String, QdrantNamedVector>),
}

#[derive(Debug, Deserialize)]
struct QdrantNamedVector {
    size: usize,
}

#[derive(Debug, Serialize)]
struct ScrollRequest {
    limit: usize,
    with_payload: bool,
    with_vector: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ScrollResponse {
    result: ScrollResult,
}

#[derive(Debug, Deserialize)]
struct ScrollResult {
    points: Vec<QdrantPoint>,
    next_page_offset: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct QdrantPoint {
    id: QdrantPointId,
    vector: QdrantVector,
    payload: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QdrantPointId {
    Num(u64),
    Uuid(String),
}

impl std::fmt::Display for QdrantPointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Num(n) => write!(f, "{n}"),
            Self::Uuid(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QdrantVector {
    Single(Vec<f32>),
    Named(HashMap<String, Vec<f32>>),
}

impl QdrantVector {
    fn into_vec(self) -> Vec<f32> {
        match self {
            Self::Single(v) => v,
            Self::Named(mut map) => {
                // Take the first named vector
                map.drain().next().map(|(_, v)| v).unwrap_or_default()
            }
        }
    }
}

#[async_trait]
impl SourceConnector for QdrantConnector {
    fn source_type(&self) -> &'static str {
        "qdrant"
    }

    async fn connect(&mut self) -> Result<()> {
        info!("Connecting to Qdrant at {}", self.config.url);

        let resp = self.request(reqwest::Method::GET, "").send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::SourceConnection(format!(
                "Qdrant connection failed: {status} - {body}"
            )));
        }

        info!("Connected to Qdrant collection: {}", self.config.collection);
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        let resp = self.request(reqwest::Method::GET, "").send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "Failed to get schema: {status} - {body}"
            )));
        }

        let info: QdrantCollectionInfo = resp.json().await?;

        let dimension = match info.result.config.params.vectors {
            QdrantVectorConfig::Single { size } => size,
            QdrantVectorConfig::Named(ref map) => map.values().next().map_or(0, |v| v.size),
        };

        let total_count = info.result.points_count.or(info.result.vectors_count);

        info!(
            "Qdrant schema: {}D vectors, {:?} total points",
            dimension, total_count
        );

        Ok(SourceSchema {
            source_type: "qdrant".to_string(),
            collection: self.config.collection.clone(),
            dimension,
            total_count,
            fields: vec![], // Qdrant doesn't expose payload schema easily
            ..Default::default()
        })
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let request_body = ScrollRequest {
            limit: batch_size,
            with_payload: true,
            with_vector: true,
            offset,
        };

        debug!("Extracting batch from Qdrant, limit={}", batch_size);

        let resp = self
            .request(reqwest::Method::POST, "/points/scroll")
            .json(&request_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "Qdrant scroll failed: {status} - {body}"
            )));
        }

        let scroll_resp: ScrollResponse = resp.json().await?;

        let points: Vec<ExtractedPoint> = scroll_resp
            .result
            .points
            .into_iter()
            .map(|p| ExtractedPoint {
                id: p.id.to_string(),
                vector: p.vector.into_vec(),
                payload: p.payload.unwrap_or_default(),
            })
            .collect();

        let has_more = scroll_resp.result.next_page_offset.is_some();

        debug!("Extracted {} points, has_more={}", points.len(), has_more);

        Ok(ExtractedBatch {
            points,
            next_offset: scroll_resp.result.next_page_offset,
            has_more,
        })
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing Qdrant connection");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qdrant_point_id_display() {
        let num_id = QdrantPointId::Num(12345);
        assert_eq!(num_id.to_string(), "12345");

        let uuid_id = QdrantPointId::Uuid("abc-123".to_string());
        assert_eq!(uuid_id.to_string(), "abc-123");
    }

    #[test]
    fn test_qdrant_vector_into_vec() {
        let single = QdrantVector::Single(vec![0.1, 0.2, 0.3]);
        assert_eq!(single.into_vec(), vec![0.1, 0.2, 0.3]);

        let named = QdrantVector::Named(HashMap::from([(
            "default".to_string(),
            vec![0.4, 0.5, 0.6],
        )]));
        assert_eq!(named.into_vec(), vec![0.4, 0.5, 0.6]);
    }

    #[test]
    fn test_scroll_request_serialization() {
        let req = ScrollRequest {
            limit: 100,
            with_payload: true,
            with_vector: true,
            offset: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"limit\":100"));
        assert!(!json.contains("offset")); // Skip serializing None
    }
}
