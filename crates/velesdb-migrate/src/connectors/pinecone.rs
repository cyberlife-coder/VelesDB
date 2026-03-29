//! Pinecone vector database connector.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, info};

use super::common::{check_response, create_http_client};
use super::{ExtractedBatch, ExtractedPoint, SourceConnector, SourceSchema};
use crate::config::PineconeConfig;
use crate::error::{Error, Result};

/// Pinecone source connector.
pub struct PineconeConnector {
    config: PineconeConfig,
    client: reqwest::Client,
    host: Option<String>,
}

impl PineconeConnector {
    /// Create a new Pinecone connector.
    #[must_use]
    pub fn new(config: PineconeConfig) -> Self {
        Self {
            config,
            client: create_http_client(),
            host: None,
        }
    }

    fn api_request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header("Api-Key", &self.config.api_key)
            .header("Content-Type", "application/json")
    }

    /// Control-plane base URL (index describe, etc.).
    fn control_plane_url(&self) -> String {
        self.config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.pinecone.io".to_string())
    }

    /// Data-plane base URL (list, fetch, stats).
    fn data_plane_url(&self) -> String {
        if let Some(base) = &self.config.base_url {
            return base.clone();
        }
        let host = self.host.as_deref().unwrap_or("localhost");
        format!("https://{host}")
    }
}

#[derive(Debug, Deserialize)]
struct IndexDescription {
    host: String,
    dimension: usize,
    metric: String,
    status: IndexStatus,
}

#[derive(Debug, Deserialize)]
struct IndexStatus {
    ready: bool,
}

#[derive(Debug, Deserialize)]
struct IndexStats {
    dimension: usize,
    #[serde(rename = "totalVectorCount")]
    total_vector_count: u64,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    vectors: Option<Vec<VectorInfo>>,
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize)]
struct VectorInfo {
    id: String,
}

#[derive(Debug, Deserialize)]
struct Pagination {
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FetchResponse {
    vectors: HashMap<String, PineconeVector>,
}

/// Pinecone sparse vector format from REST API.
#[derive(Debug, Deserialize)]
struct PineconeSparseValues {
    indices: Vec<u32>,
    values: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct PineconeVector {
    id: String,
    values: Vec<f32>,
    metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(rename = "sparseValues", default)]
    sparse_values: Option<PineconeSparseValues>,
}

#[async_trait]
impl SourceConnector for PineconeConnector {
    fn source_type(&self) -> &'static str {
        "pinecone"
    }

    async fn connect(&mut self) -> Result<()> {
        info!("Connecting to Pinecone index: {}", self.config.index);

        // Get index description to find the host
        let base = self.control_plane_url();
        let url = format!("{base}/indexes/{}", self.config.index);

        let resp = self.api_request(reqwest::Method::GET, &url).send().await?;
        let checked = check_response(resp, "Pinecone", "connect").await?;

        let desc: IndexDescription = checked.json().await?;

        if !desc.status.ready {
            return Err(Error::SourceConnection(
                "Pinecone index is not ready".to_string(),
            ));
        }

        self.host = Some(desc.host);
        info!(
            "Connected to Pinecone index: {} ({}D, {})",
            self.config.index, desc.dimension, desc.metric
        );

        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        let _host = self
            .host
            .as_ref()
            .ok_or_else(|| Error::SourceConnection("Not connected to Pinecone".to_string()))?;

        let base = self.data_plane_url();
        let url = format!("{base}/describe_index_stats");
        let resp = self
            .api_request(reqwest::Method::POST, &url)
            .json(&serde_json::json!({}))
            .send()
            .await?;

        let checked = check_response(resp, "Pinecone", "stats").await?;

        let stats: IndexStats = checked.json().await?;

        let count = if stats.total_vector_count > 0 {
            Some(stats.total_vector_count)
        } else {
            None
        };

        info!("Pinecone index: {}D, {:?} vectors", stats.dimension, count);

        Ok(SourceSchema {
            source_type: "pinecone".to_string(),
            collection: self.config.index.clone(),
            dimension: stats.dimension,
            total_count: count,
            fields: vec![],
            ..Default::default()
        })
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let _host = self
            .host
            .as_ref()
            .ok_or_else(|| Error::SourceConnection("Not connected to Pinecone".to_string()))?;

        let pagination_token = offset.and_then(|v| v.as_str().map(String::from));

        // First, list vector IDs
        let base = self.data_plane_url();
        let list_url = format!("{base}/vectors/list");

        let mut list_params: Vec<(&str, String)> = vec![("limit", batch_size.to_string())];
        if let Some(ref token) = pagination_token {
            list_params.push(("paginationToken", token.clone()));
        }
        if let Some(ref ns) = self.config.namespace {
            list_params.push(("namespace", ns.clone()));
        }

        debug!("Listing Pinecone vectors, limit={}", batch_size);

        let resp = self
            .api_request(reqwest::Method::GET, &list_url)
            .query(&list_params)
            .send()
            .await?;

        let checked = check_response(resp, "Pinecone", "list").await?;

        let list_resp: ListResponse = checked.json().await?;

        let ids: Vec<String> = list_resp
            .vectors
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.id)
            .collect();

        if ids.is_empty() {
            return Ok(ExtractedBatch {
                points: vec![],
                next_offset: None,
                has_more: false,
            });
        }

        // Fetch full vectors — Pinecone expects repeated `ids` query params
        let fetch_url = format!("{base}/vectors/fetch");
        let mut fetch_params: Vec<(&str, String)> =
            ids.iter().map(|id| ("ids", id.clone())).collect();
        if let Some(ref ns) = self.config.namespace {
            fetch_params.push(("namespace", ns.clone()));
        }
        let resp = self
            .api_request(reqwest::Method::GET, &fetch_url)
            .query(&fetch_params)
            .send()
            .await?;

        let checked = check_response(resp, "Pinecone", "fetch").await?;

        let fetch_resp: FetchResponse = checked.json().await?;

        let points: Vec<ExtractedPoint> = fetch_resp
            .vectors
            .into_values()
            .map(|v| {
                let sparse = v.sparse_values.and_then(|sv| {
                    if sv.indices.len() == sv.values.len() && !sv.indices.is_empty() {
                        Some(sv.indices.into_iter().zip(sv.values).collect())
                    } else {
                        None
                    }
                });
                ExtractedPoint {
                    id: v.id,
                    vector: v.values,
                    payload: v.metadata.unwrap_or_default(),
                    sparse_vector: sparse,
                }
            })
            .collect();

        let next_offset = list_resp
            .pagination
            .and_then(|p| p.next)
            .map(serde_json::Value::String);

        let has_more = next_offset.is_some();

        debug!("Extracted {} points from Pinecone", points.len());

        Ok(ExtractedBatch {
            points,
            next_offset,
            has_more,
        })
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing Pinecone connection");
        self.host = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pinecone_connector_new() {
        let config = PineconeConfig {
            api_key: "test-key".to_string(),
            environment: "us-east-1".to_string(),
            index: "test-index".to_string(),
            namespace: None,
            base_url: None,
        };

        let connector = PineconeConnector::new(config);
        assert_eq!(connector.source_type(), "pinecone");
        assert!(connector.host.is_none());
    }

    #[test]
    fn test_list_query_params_construction() {
        let batch_size: usize = 100;
        let pagination_token: Option<String> = Some("tok-abc".to_string());
        let namespace: Option<String> = Some("ns1".to_string());

        let mut params: Vec<(&str, String)> = vec![("limit", batch_size.to_string())];
        if let Some(ref token) = pagination_token {
            params.push(("paginationToken", token.clone()));
        }
        if let Some(ref ns) = namespace {
            params.push(("namespace", ns.clone()));
        }

        assert_eq!(params.len(), 3);
        assert_eq!(params[0], ("limit", "100".to_string()));
        assert_eq!(params[1], ("paginationToken", "tok-abc".to_string()));
        assert_eq!(params[2], ("namespace", "ns1".to_string()));
    }

    #[test]
    fn test_list_query_params_without_optional_fields() {
        let batch_size: usize = 50;
        let pagination_token: Option<String> = None;
        let namespace: Option<String> = None;

        let mut params: Vec<(&str, String)> = vec![("limit", batch_size.to_string())];
        if let Some(ref token) = pagination_token {
            params.push(("paginationToken", token.clone()));
        }
        if let Some(ref ns) = namespace {
            params.push(("namespace", ns.clone()));
        }

        assert_eq!(params.len(), 1);
        assert_eq!(params[0], ("limit", "50".to_string()));
    }

    #[test]
    fn test_fetch_query_params_with_namespace() {
        let ids = ["id-1".to_string(), "id-2".to_string(), "id-3".to_string()];
        let namespace: Option<String> = Some("ns1".to_string());

        let mut fetch_params: Vec<(&str, String)> =
            ids.iter().map(|id| ("ids", id.clone())).collect();
        if let Some(ref ns) = namespace {
            fetch_params.push(("namespace", ns.clone()));
        }

        assert_eq!(fetch_params.len(), 4);
        assert_eq!(fetch_params[0], ("ids", "id-1".to_string()));
        assert_eq!(fetch_params[1], ("ids", "id-2".to_string()));
        assert_eq!(fetch_params[2], ("ids", "id-3".to_string()));
        assert_eq!(fetch_params[3], ("namespace", "ns1".to_string()));
    }

    #[test]
    fn test_pinecone_vector_with_sparse() {
        let json = r#"{
            "id": "vec-1",
            "values": [0.1, 0.2],
            "sparseValues": {
                "indices": [0, 5, 11],
                "values": [0.5, 0.3, 0.8]
            }
        }"#;

        let v: PineconeVector = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, "vec-1");
        assert_eq!(v.values, vec![0.1, 0.2]);

        let sv = v.sparse_values.expect("sparse_values should be present");
        assert_eq!(sv.indices, vec![0, 5, 11]);
        assert_eq!(sv.values, vec![0.5, 0.3, 0.8]);
    }

    #[test]
    fn test_pinecone_vector_without_sparse() {
        let json = r#"{
            "id": "vec-2",
            "values": [0.4, 0.5]
        }"#;

        let v: PineconeVector = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, "vec-2");
        assert!(v.sparse_values.is_none());
    }

    #[test]
    fn test_pinecone_sparse_extraction_in_point() {
        let v = PineconeVector {
            id: "vec-3".to_string(),
            values: vec![1.0, 2.0, 3.0],
            metadata: None,
            sparse_values: Some(PineconeSparseValues {
                indices: vec![2, 7],
                values: vec![0.9, 0.1],
            }),
        };

        let sparse = v.sparse_values.and_then(|sv| {
            if sv.indices.len() == sv.values.len() && !sv.indices.is_empty() {
                Some(sv.indices.into_iter().zip(sv.values).collect::<Vec<_>>())
            } else {
                None
            }
        });

        let point = ExtractedPoint {
            id: v.id,
            vector: v.values,
            payload: v.metadata.unwrap_or_default(),
            sparse_vector: sparse,
        };

        assert_eq!(point.id, "vec-3");
        let sv = point.sparse_vector.expect("should have sparse vector");
        assert_eq!(sv, vec![(2, 0.9), (7, 0.1)]);
    }
}
