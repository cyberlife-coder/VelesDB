//! Milvus vector database connector.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use super::common::{
    build_numeric_offset_batch, check_response, create_http_client, extract_id_from_value,
};
use super::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::config::MilvusConfig;
use crate::error::{Error, Result};

/// Milvus source connector.
pub struct MilvusConnector {
    config: MilvusConfig,
    client: reqwest::Client,
    vector_field: Option<String>,
    cached_schema: Option<SourceSchema>,
}

impl MilvusConnector {
    /// Create a new Milvus connector.
    #[must_use]
    pub fn new(config: MilvusConfig) -> Self {
        Self {
            config,
            client: create_http_client(),
            vector_field: None,
            cached_schema: None,
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

#[derive(Debug, Serialize)]
struct DescribeIndexRequest<'a> {
    #[serde(rename = "collectionName")]
    collection_name: &'a str,
    #[serde(rename = "indexName")]
    index_name: &'a str,
}

#[derive(Debug, Deserialize)]
struct IndexDescription {
    #[serde(rename = "metricType")]
    metric_type: Option<String>,
}

#[async_trait]
impl SourceConnector for MilvusConnector {
    fn source_type(&self) -> &'static str {
        "milvus"
    }

    async fn connect(&mut self) -> Result<()> {
        crate::connectors::common::validate_url(&self.config.url)?;

        info!("Connecting to Milvus at {}", self.config.url);

        let resp = self
            .request(reqwest::Method::GET, "/collections/has")
            .query(&[("collectionName", &self.config.collection)])
            .send()
            .await?;

        let checked = check_response(resp, "Milvus", "connect").await?;

        let result: MilvusResponse<bool> = checked.json().await?;

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

        self.fetch_and_cache_schema().await?;
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        crate::connectors::common::cached_schema(&self.cached_schema)
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let current_offset = usize::try_from(
            offset
                .as_ref()
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(usize::MAX);

        let vector_field = self
            .vector_field
            .as_deref()
            .ok_or_else(|| Error::SourceConnection("Not connected to Milvus".to_string()))?
            .to_owned();

        let schema = self.get_schema().await?;
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

        let resp = check_response(resp, "Milvus", "query").await?;

        let result: MilvusResponse<Vec<HashMap<String, serde_json::Value>>> = resp.json().await?;

        let rows = result.data.unwrap_or_default();
        let mut points = Vec::with_capacity(rows.len());

        for mut row in rows {
            let id = extract_id_from_value(row.remove("id"));

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
                sparse_vector: None,
            });
        }

        debug!("Extracted {} rows from Milvus", points.len());

        Ok(build_numeric_offset_batch(
            points,
            batch_size,
            current_offset as u64,
        ))
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing Milvus connection");
        Ok(())
    }
}

impl MilvusConnector {
    /// Detect the first vector-typed field in the Milvus collection schema.
    fn find_vector_field(schema: &CollectionSchema) -> Result<(String, usize)> {
        for field in &schema.fields {
            let ft = field.field_type.to_ascii_uppercase();
            if ft.contains("VECTOR") {
                let dim = field.params.as_ref().and_then(|p| p.dim).unwrap_or(0);
                return Ok((field.name.clone(), dim));
            }
        }
        Err(Error::SchemaMismatch("No vector field found".to_string()))
    }

    /// Normalise a Milvus `metricType` identifier to the VelesDB core
    /// vocabulary so `Pipeline::check_metric_fidelity` can compare it
    /// against a destination collection's metric.
    ///
    /// Milvus exposes `L2`, `IP`, `COSINE`, `HAMMING`, `JACCARD`, and
    /// (legacy) `TANIMOTO`. VelesDB core uses `euclidean`, `dot`,
    /// `cosine`, `hamming`, `jaccard`. `L2` maps to `euclidean`, `IP`
    /// (inner product) maps to `dot`. Unknown labels such as
    /// `TANIMOTO` are lowercased and returned verbatim so mismatch
    /// errors remain actionable instead of being silently dropped.
    fn normalise_milvus_metric(raw: &str) -> String {
        let lower = raw.to_ascii_lowercase();
        match lower.as_str() {
            "l2" => "euclidean".to_string(),
            "ip" => "dot".to_string(),
            _ => lower,
        }
    }

    /// Best-effort retrieval of the distance metric configured on the
    /// vector-field index. Milvus v2 REST does not return the metric in
    /// `/collections/describe`; it is stored on the associated index
    /// and accessible via `POST /v2/vectordb/indexes/describe`. If the
    /// call fails (older Milvus versions, permission issues, or the
    /// index not yet built), we log a warning and return `None` so the
    /// rest of the schema extraction can proceed — `check_metric_fidelity`
    /// will skip validation for this source rather than blocking
    /// migration.
    async fn fetch_index_metric(&self, vector_field: &str) -> Option<String> {
        let body = DescribeIndexRequest {
            collection_name: &self.config.collection,
            index_name: vector_field,
        };

        let resp = match self
            .request(reqwest::Method::POST, "/indexes/describe")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    error = %e,
                    "Milvus /indexes/describe request failed; metric \
                     fidelity check will be skipped for this source"
                );
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!(
                status = %resp.status(),
                "Milvus /indexes/describe returned non-success; metric \
                 fidelity check will be skipped for this source"
            );
            return None;
        }

        let parsed: MilvusResponse<IndexDescription> = match resp.json().await {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    error = %e,
                    "Milvus /indexes/describe response failed to parse; \
                     metric fidelity check will be skipped for this source"
                );
                return None;
            }
        };

        if parsed.code != 0 {
            warn!(
                code = parsed.code,
                message = ?parsed.message,
                "Milvus /indexes/describe returned error code; metric \
                 fidelity check will be skipped for this source"
            );
            return None;
        }

        parsed
            .data
            .and_then(|d| d.metric_type)
            .map(|raw| Self::normalise_milvus_metric(&raw))
    }

    /// Fetch the collection schema from Milvus and cache it locally.
    async fn fetch_and_cache_schema(&mut self) -> Result<()> {
        let resp = self
            .request(reqwest::Method::GET, "/collections/describe")
            .query(&[("collectionName", &self.config.collection)])
            .send()
            .await?;

        let resp = check_response(resp, "Milvus", "describe").await?;
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

        let metric = self.fetch_index_metric(&vector_field).await;

        info!(
            "Milvus collection '{}': {}D vectors, metric={:?}, {:?} rows",
            self.config.collection, dimension, metric, total_count
        );

        self.vector_field = Some(vector_field);
        self.cached_schema = Some(SourceSchema {
            source_type: "milvus".to_string(),
            collection: self.config.collection.clone(),
            dimension,
            total_count,
            fields,
            metric,
            ..Default::default()
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalise_milvus_metric_maps_l2_to_euclidean() {
        // Milvus reports 'L2' for squared-L2 distance; VelesDB core
        // uses 'euclidean'. The mapping is what allows
        // check_metric_fidelity to honestly compare a Milvus source
        // against a core collection created with metric: "euclidean".
        assert_eq!(MilvusConnector::normalise_milvus_metric("L2"), "euclidean");
        assert_eq!(MilvusConnector::normalise_milvus_metric("l2"), "euclidean");
    }

    #[test]
    fn test_normalise_milvus_metric_maps_ip_to_dot() {
        // Milvus 'IP' (inner product) maps to VelesDB's 'dot'.
        assert_eq!(MilvusConnector::normalise_milvus_metric("IP"), "dot");
        assert_eq!(MilvusConnector::normalise_milvus_metric("ip"), "dot");
    }

    #[test]
    fn test_normalise_milvus_metric_lowercases_known_values() {
        assert_eq!(MilvusConnector::normalise_milvus_metric("COSINE"), "cosine");
        assert_eq!(
            MilvusConnector::normalise_milvus_metric("HAMMING"),
            "hamming"
        );
        assert_eq!(
            MilvusConnector::normalise_milvus_metric("JACCARD"),
            "jaccard"
        );
    }

    #[test]
    fn test_normalise_milvus_metric_preserves_unknown_values() {
        // TANIMOTO is a legacy Milvus metric not supported by VelesDB
        // core — preserved verbatim so mismatch errors stay actionable.
        assert_eq!(
            MilvusConnector::normalise_milvus_metric("TANIMOTO"),
            "tanimoto"
        );
    }

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

    #[test]
    fn test_connect_rejects_file_url() {
        assert!(crate::connectors::common::validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_find_vector_field_detects_float_vector() {
        let schema = CollectionSchema {
            fields: vec![
                FieldSchema {
                    name: "id".to_string(),
                    field_type: "Int64".to_string(),
                    is_primary_key: Some(true),
                    params: None,
                },
                FieldSchema {
                    name: "embedding".to_string(),
                    field_type: "FloatVector".to_string(),
                    is_primary_key: None,
                    params: Some(FieldParams { dim: Some(128) }),
                },
            ],
        };
        let (name, dim) =
            MilvusConnector::find_vector_field(&schema).expect("test: should detect FloatVector");
        assert_eq!(name, "embedding");
        assert_eq!(dim, 128);
    }

    #[test]
    fn test_find_vector_field_detects_float_vector_uppercase() {
        let schema = CollectionSchema {
            fields: vec![FieldSchema {
                name: "vec".to_string(),
                field_type: "FLOAT_VECTOR".to_string(),
                is_primary_key: None,
                params: Some(FieldParams { dim: Some(768) }),
            }],
        };
        let (name, dim) =
            MilvusConnector::find_vector_field(&schema).expect("test: FLOAT_VECTOR uppercase");
        assert_eq!(name, "vec");
        assert_eq!(dim, 768);
    }

    #[test]
    fn test_find_vector_field_returns_error_when_no_vector_field() {
        let schema = CollectionSchema {
            fields: vec![
                FieldSchema {
                    name: "id".to_string(),
                    field_type: "Int64".to_string(),
                    is_primary_key: Some(true),
                    params: None,
                },
                FieldSchema {
                    name: "name".to_string(),
                    field_type: "VarChar".to_string(),
                    is_primary_key: None,
                    params: None,
                },
            ],
        };
        assert!(MilvusConnector::find_vector_field(&schema).is_err());
    }
}
