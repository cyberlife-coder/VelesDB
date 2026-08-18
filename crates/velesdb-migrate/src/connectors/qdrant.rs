//! Qdrant vector database connector.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use super::common::{check_response, create_http_client, normalise_metric};
use super::{ExtractedBatch, ExtractedPoint, SourceConnector, SourceSchema};
use crate::config::QdrantConfig;
use crate::error::Result;

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
            client: create_http_client(),
        }
    }

    /// Normalise a Qdrant distance identifier to the VelesDB core
    /// vocabulary so `Pipeline::check_metric_fidelity` can compare it
    /// against a destination collection's metric.
    ///
    /// Qdrant exposes `Cosine`, `Euclid`, `Dot`, and `Manhattan`
    /// (1.8+). VelesDB core uses `cosine`, `euclidean`, `dot`,
    /// `hamming`, `jaccard`. `Euclid` is mapped to `euclidean`;
    /// unknown values (e.g. `manhattan`) are lowercased and returned
    /// verbatim so mismatch errors stay actionable instead of being
    /// silently dropped.
    fn normalise_qdrant_metric(raw: &str) -> String {
        normalise_metric(raw, &[("euclid", "euclidean")])
    }

    /// Pick the "primary" named vector from a multi-vector Qdrant
    /// collection. Returns its `(dimension, raw_metric)`.
    ///
    /// Qdrant 1.7+ supports multiple named vectors per collection
    /// (e.g. `default`, `secondary`, `text_embedding`). For
    /// migration purposes we need to pick exactly one — the rest
    /// of the pipeline assumes a single primary vector per source.
    /// Selection policy, in order of preference:
    /// 1. The entry named `"default"` if present — Qdrant's
    ///    implicit name when a single unnamed vector is upgraded.
    /// 2. Otherwise the lexicographically first entry, so the
    ///    result is deterministic across runs (HashMap iteration
    ///    order is not).
    ///
    /// Returns `(0, None)` only for an empty map, which should
    /// never happen on a well-formed Qdrant response but is
    /// handled defensively.
    fn pick_named_vector(map: &HashMap<String, QdrantNamedVector>) -> (usize, Option<String>) {
        if let Some(default) = map.get("default") {
            return (default.size, default.distance.clone());
        }
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        keys.first()
            .and_then(|k| map.get(*k))
            .map_or((0, None), |v| (v.size, v.distance.clone()))
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
    Single {
        size: usize,
        #[serde(default)]
        distance: Option<String>,
    },
    Named(HashMap<String, QdrantNamedVector>),
}

#[derive(Debug, Deserialize)]
struct QdrantNamedVector {
    size: usize,
    #[serde(default)]
    distance: Option<String>,
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
                        if crate::connectors::common::is_valid_sparse_vector(
                            &sv.indices,
                            &sv.values,
                        ) {
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
        crate::connectors::common::validate_url(&self.config.url)?;

        info!("Connecting to Qdrant at {}", self.config.url);

        let resp = self.request(reqwest::Method::GET, "").send().await?;
        check_response(resp, "Qdrant", "connect").await?;

        info!("Connected to Qdrant collection: {}", self.config.collection);
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        let resp = self.request(reqwest::Method::GET, "").send().await?;
        let checked = check_response(resp, "Qdrant", "get_schema").await?;

        let info: QdrantCollectionInfo = checked.json().await?;

        let (dimension, raw_metric) = match info.result.config.params.vectors {
            QdrantVectorConfig::Single { size, ref distance } => (size, distance.clone()),
            QdrantVectorConfig::Named(ref map) => Self::pick_named_vector(map),
        };
        let metric = raw_metric.as_deref().map(Self::normalise_qdrant_metric);

        let total_count = info.result.points_count.or(info.result.vectors_count);

        info!(
            "Qdrant schema: {}D vectors, metric={:?}, {:?} total points",
            dimension, metric, total_count
        );

        Ok(SourceSchema {
            source_type: "qdrant".to_string(),
            collection: self.config.collection.clone(),
            dimension,
            total_count,
            fields: vec![], // Qdrant doesn't expose payload schema easily
            metric,
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

        let checked = check_response(resp, "Qdrant", "scroll").await?;

        let scroll_resp: ScrollResponse = checked.json().await?;

        let points: Vec<ExtractedPoint> = scroll_resp
            .result
            .points
            .into_iter()
            .filter_map(|p| {
                let sparse = p.vector.extract_sparse();
                let dense = p.vector.into_dense();
                if dense.is_empty() {
                    warn!(
                        point_id = %p.id,
                        "Skipping point with no dense vector \
                         (sparse-only points are not supported)"
                    );
                    return None;
                }
                Some(ExtractedPoint {
                    id: p.id.to_string(),
                    vector: dense,
                    payload: p.payload.unwrap_or_default(),
                    sparse_vector: sparse,
                })
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
#[path = "qdrant_tests.rs"]
mod tests;
