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

/// Qdrant sparse vector format from REST API.
#[derive(Debug, Deserialize)]
struct QdrantSparseVector {
    indices: Vec<u32>,
    values: Vec<f32>,
}

/// A named vector entry can be dense (array) or sparse (object with indices/values).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QdrantNamedVectorValue {
    Dense(Vec<f32>),
    Sparse(QdrantSparseVector),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QdrantVector {
    Single(Vec<f32>),
    Named(HashMap<String, QdrantNamedVectorValue>),
}

impl QdrantVector {
    /// Extract the first sparse vector from a Named map, if present.
    ///
    /// Returns `None` for `Single` vectors, or if no valid sparse entry exists.
    /// A sparse entry is valid only when `indices` and `values` have equal,
    /// non-zero lengths.
    fn extract_sparse(&self) -> Option<Vec<(u32, f32)>> {
        match self {
            Self::Single(_) => None,
            Self::Named(map) => {
                for value in map.values() {
                    if let QdrantNamedVectorValue::Sparse(sv) = value {
                        if sv.indices.len() == sv.values.len() && !sv.indices.is_empty() {
                            return Some(
                                sv.indices
                                    .iter()
                                    .copied()
                                    .zip(sv.values.iter().copied())
                                    .collect(),
                            );
                        }
                    }
                }
                None
            }
        }
    }

    /// Consume the vector and return the first dense embedding.
    ///
    /// For `Named` maps, sparse entries are skipped. Returns an empty vec
    /// if no dense vector is found.
    fn into_dense(self) -> Vec<f32> {
        match self {
            Self::Single(v) => v,
            Self::Named(map) => {
                for (_, value) in map {
                    if let QdrantNamedVectorValue::Dense(v) = value {
                        return v;
                    }
                }
                Vec::new()
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
            .map(|p| {
                let sparse = p.vector.extract_sparse();
                ExtractedPoint {
                    id: p.id.to_string(),
                    vector: p.vector.into_dense(),
                    payload: p.payload.unwrap_or_default(),
                    sparse_vector: sparse,
                }
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
    fn test_qdrant_vector_into_dense() {
        let single = QdrantVector::Single(vec![0.1, 0.2, 0.3]);
        assert_eq!(single.into_dense(), vec![0.1, 0.2, 0.3]);

        let named = QdrantVector::Named(HashMap::from([(
            "default".to_string(),
            QdrantNamedVectorValue::Dense(vec![0.4, 0.5, 0.6]),
        )]));
        assert_eq!(named.into_dense(), vec![0.4, 0.5, 0.6]);
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

    #[test]
    fn test_qdrant_sparse_vector_deserialization() {
        let json = r#"{"indices":[10,45],"values":[0.5,0.3]}"#;
        let sv: QdrantSparseVector = serde_json::from_str(json).unwrap();
        assert_eq!(sv.indices, vec![10, 45]);
        assert_eq!(sv.values, vec![0.5, 0.3]);
    }

    #[test]
    fn test_qdrant_named_vector_with_sparse() {
        let named = QdrantVector::Named(HashMap::from([
            (
                "dense".to_string(),
                QdrantNamedVectorValue::Dense(vec![0.1, 0.2]),
            ),
            (
                "sparse".to_string(),
                QdrantNamedVectorValue::Sparse(QdrantSparseVector {
                    indices: vec![3, 7, 42],
                    values: vec![0.9, 0.1, 0.5],
                }),
            ),
        ]));

        let sparse = named.extract_sparse();
        assert!(sparse.is_some());
        let pairs = sparse.unwrap();
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].0, 3);
        assert!((pairs[0].1 - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_qdrant_single_vector_no_sparse() {
        let single = QdrantVector::Single(vec![1.0, 2.0, 3.0]);
        assert!(single.extract_sparse().is_none());
    }

    #[test]
    fn test_qdrant_sparse_mismatched_lengths() {
        let named = QdrantVector::Named(HashMap::from([(
            "bad_sparse".to_string(),
            QdrantNamedVectorValue::Sparse(QdrantSparseVector {
                indices: vec![1, 2, 3],
                values: vec![0.5, 0.3],
            }),
        )]));

        assert!(named.extract_sparse().is_none());
    }
}
