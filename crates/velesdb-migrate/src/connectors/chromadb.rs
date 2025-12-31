//! `ChromaDB` vector database connector.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

use super::{ExtractedBatch, ExtractedPoint, SourceConnector, SourceSchema};
use crate::config::ChromaDBConfig;
use crate::error::{Error, Result};

/// `ChromaDB` source connector.
pub struct ChromaDBConnector {
    config: ChromaDBConfig,
    client: reqwest::Client,
    collection_id: Option<String>,
}

impl ChromaDBConnector {
    /// Create a new `ChromaDB` connector.
    #[must_use]
    pub fn new(config: ChromaDBConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            collection_id: None,
        }
    }

    fn base_url(&self) -> String {
        let base = self.config.url.trim_end_matches('/');
        match (&self.config.tenant, &self.config.database) {
            (Some(t), Some(d)) => format!("{base}/api/v1/tenants/{t}/databases/{d}"),
            _ => format!("{base}/api/v1"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CollectionInfo {
    id: String,
    name: String,
    #[allow(dead_code)] // Parsed from JSON, reserved for future use
    metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct GetRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<usize>,
    include: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GetResponse {
    ids: Vec<String>,
    embeddings: Option<Vec<Vec<f32>>>,
    metadatas: Option<Vec<Option<HashMap<String, serde_json::Value>>>>,
    documents: Option<Vec<Option<String>>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Reserved for future count endpoint
struct CountResponse {
    count: u64,
}

#[async_trait]
impl SourceConnector for ChromaDBConnector {
    fn source_type(&self) -> &'static str {
        "chromadb"
    }

    async fn connect(&mut self) -> Result<()> {
        info!("Connecting to ChromaDB at {}", self.config.url);

        // Get collection by name
        let url = format!("{}/collections/{}", self.base_url(), self.config.collection);

        let resp = self
            .client
            .get(&url)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::SourceConnection(format!(
                "ChromaDB connection failed: {status} - {body}"
            )));
        }

        let collection: CollectionInfo = resp.json().await?;
        self.collection_id = Some(collection.id.clone());

        info!(
            "Connected to ChromaDB collection: {} ({})",
            collection.name, collection.id
        );

        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        let collection_id = self
            .collection_id
            .as_ref()
            .ok_or_else(|| Error::SourceConnection("Not connected to ChromaDB".to_string()))?;

        // Get count
        let url = format!("{}/collections/{}/count", self.base_url(), collection_id);

        let resp = self.client.get(&url).send().await?;

        let total_count = if resp.status().is_success() {
            let count: u64 = resp.json().await?;
            Some(count)
        } else {
            None
        };

        // ChromaDB doesn't expose dimension directly, we need to fetch one embedding
        let mut dimension = 0;

        let peek_url = format!("{}/collections/{}/get", self.base_url(), collection_id);
        let peek_req = GetRequest {
            ids: None,
            limit: Some(1),
            offset: None,
            include: vec!["embeddings".to_string()],
        };

        let resp = self.client.post(&peek_url).json(&peek_req).send().await?;

        if resp.status().is_success() {
            let data: GetResponse = resp.json().await?;
            if let Some(embeddings) = data.embeddings {
                if let Some(first) = embeddings.first() {
                    dimension = first.len();
                }
            }
        }

        info!(
            "ChromaDB collection: {}D vectors, {:?} documents",
            dimension, total_count
        );

        Ok(SourceSchema {
            source_type: "chromadb".to_string(),
            collection: self.config.collection.clone(),
            dimension,
            total_count,
            fields: vec![],
            ..Default::default()
        })
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let collection_id = self
            .collection_id
            .as_ref()
            .ok_or_else(|| Error::SourceConnection("Not connected to ChromaDB".to_string()))?;

        let current_offset = offset
            .as_ref()
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;

        let url = format!("{}/collections/{}/get", self.base_url(), collection_id);

        let req = GetRequest {
            ids: None,
            limit: Some(batch_size),
            offset: Some(current_offset),
            include: vec![
                "embeddings".to_string(),
                "metadatas".to_string(),
                "documents".to_string(),
            ],
        };

        debug!(
            "Extracting batch from ChromaDB, offset={}, limit={}",
            current_offset, batch_size
        );

        let resp = self.client.post(&url).json(&req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "ChromaDB get failed: {status} - {body}"
            )));
        }

        let data: GetResponse = resp.json().await?;

        let embeddings = data.embeddings.unwrap_or_default();
        let metadatas = data.metadatas.unwrap_or_default();
        let documents = data.documents.unwrap_or_default();

        let mut points = Vec::with_capacity(data.ids.len());

        for (i, id) in data.ids.into_iter().enumerate() {
            let vector = embeddings.get(i).cloned().unwrap_or_default();

            let mut payload: HashMap<String, serde_json::Value> = metadatas
                .get(i)
                .and_then(std::clone::Clone::clone)
                .unwrap_or_default();

            if let Some(Some(doc)) = documents.get(i) {
                payload.insert(
                    "document".to_string(),
                    serde_json::Value::String(doc.clone()),
                );
            }

            points.push(ExtractedPoint {
                id,
                vector,
                payload,
            });
        }

        let has_more = points.len() == batch_size;
        let next_offset = if has_more {
            Some(serde_json::json!(current_offset + points.len()))
        } else {
            None
        };

        debug!("Extracted {} documents from ChromaDB", points.len());

        Ok(ExtractedBatch {
            points,
            next_offset,
            has_more,
        })
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing ChromaDB connection");
        self.collection_id = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chromadb_connector_new() {
        let config = ChromaDBConfig {
            url: "http://localhost:8000".to_string(),
            collection: "test".to_string(),
            tenant: None,
            database: None,
        };

        let connector = ChromaDBConnector::new(config);
        assert_eq!(connector.source_type(), "chromadb");
        assert!(connector.collection_id.is_none());
    }

    #[test]
    fn test_get_request_serialization() {
        let req = GetRequest {
            ids: None,
            limit: Some(100),
            offset: Some(0),
            include: vec!["embeddings".to_string()],
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"limit\":100"));
        assert!(json.contains("\"include\":[\"embeddings\"]"));
        assert!(!json.contains("\"ids\"")); // Skip None
    }

    #[test]
    fn test_base_url_with_tenant() {
        let config = ChromaDBConfig {
            url: "http://localhost:8000".to_string(),
            collection: "test".to_string(),
            tenant: Some("my_tenant".to_string()),
            database: Some("my_db".to_string()),
        };

        let connector = ChromaDBConnector::new(config);
        let url = connector.base_url();
        assert!(url.contains("tenants/my_tenant/databases/my_db"));
    }
}
