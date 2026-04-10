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
    /// Index definitions included in the describe response.
    ///
    /// Milvus v2 REST (`POST /v2/vectordb/collections/describe`)
    /// returns every index attached to the collection in this array.
    /// Each entry carries `fieldName`, `indexName`, and `metricType`
    /// — the metric we want to forward into `SourceSchema.metric`.
    /// Absent on older Milvus versions that pre-date the unified
    /// describe response (pre-2.3.x).
    #[serde(default)]
    indexes: Vec<IndexInfo>,
}

#[derive(Debug, Deserialize)]
struct FieldSchema {
    name: String,
    #[serde(rename = "type")]
    field_type: String,
    #[serde(rename = "isPrimaryKey")]
    is_primary_key: Option<bool>,
    /// Milvus v2 REST returns field params as an array of
    /// `{key, value}` objects (where `value` is a stringified
    /// integer) rather than a flat map. We parse the array raw and
    /// extract `dim` via helper because `#[serde(deserialize_with)]`
    /// on a nested custom shape would obscure the failure mode.
    #[serde(default)]
    params: Vec<FieldParam>,
}

#[derive(Debug, Deserialize)]
struct FieldParam {
    key: String,
    value: String,
}

impl FieldSchema {
    /// Extract the vector dimension from the `dim` entry of the
    /// `params` array. Returns 0 when the entry is missing or the
    /// value cannot be parsed — consistent with the previous
    /// behaviour for non-vector fields.
    fn dimension(&self) -> usize {
        self.params
            .iter()
            .find(|p| p.key == "dim")
            .and_then(|p| p.value.parse::<usize>().ok())
            .unwrap_or(0)
    }
}

/// Index definition returned inside `CollectionSchema.indexes` by
/// `POST /v2/vectordb/collections/describe`. Every vector field has
/// a corresponding entry whose `metricType` field carries the
/// distance metric the operator configured at index creation time.
#[derive(Debug, Deserialize)]
struct IndexInfo {
    #[serde(rename = "fieldName")]
    field_name: String,
    #[serde(rename = "metricType")]
    metric_type: Option<String>,
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
                return Ok((field.name.clone(), field.dimension()));
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

    /// Extract the distance metric for the given vector field from
    /// a parsed `/collections/describe` response.
    ///
    /// The Milvus v2 REST `POST /v2/vectordb/collections/describe`
    /// call already returns an `indexes` array with every index
    /// attached to the collection — each entry carries `fieldName`,
    /// `indexName`, and `metricType`. We simply locate the index
    /// whose `fieldName` matches the detected vector field and
    /// forward its `metricType`, normalised to the VelesDB core
    /// vocabulary. This avoids the dead-end of chasing a separate
    /// `/indexes/describe` endpoint that the v2 REST surface does
    /// not actually expose.
    ///
    /// Returns `None` when the collection has no index yet (newly
    /// created, still building, or flushed without indexing) — in
    /// that case `check_metric_fidelity` will skip validation rather
    /// than blocking migration. This is the honest best-effort
    /// behaviour we want for unindexed sources.
    fn extract_index_metric(schema: &CollectionSchema, vector_field: &str) -> Option<String> {
        schema
            .indexes
            .iter()
            .find(|idx| idx.field_name == vector_field)
            .and_then(|idx| idx.metric_type.as_deref())
            .map(Self::normalise_milvus_metric)
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

        let metric = Self::extract_index_metric(&schema, &vector_field);

        if metric.is_none() {
            warn!(
                collection = %self.config.collection,
                vector_field = %vector_field,
                "Milvus describe response did not include an index with \
                 metricType for the vector field — metric fidelity check \
                 will be skipped for this source. This typically means \
                 the index has not been created yet or is still building."
            );
        }

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

    fn dim_param(dim: usize) -> Vec<FieldParam> {
        vec![FieldParam {
            key: "dim".to_string(),
            value: dim.to_string(),
        }]
    }

    #[test]
    fn test_find_vector_field_detects_float_vector() {
        let schema = CollectionSchema {
            fields: vec![
                FieldSchema {
                    name: "id".to_string(),
                    field_type: "Int64".to_string(),
                    is_primary_key: Some(true),
                    params: vec![],
                },
                FieldSchema {
                    name: "embedding".to_string(),
                    field_type: "FloatVector".to_string(),
                    is_primary_key: None,
                    params: dim_param(128),
                },
            ],
            indexes: vec![],
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
                params: dim_param(768),
            }],
            indexes: vec![],
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
                    params: vec![],
                },
                FieldSchema {
                    name: "name".to_string(),
                    field_type: "VarChar".to_string(),
                    is_primary_key: None,
                    params: vec![],
                },
            ],
            indexes: vec![],
        };
        assert!(MilvusConnector::find_vector_field(&schema).is_err());
    }

    #[test]
    fn test_extract_index_metric_matches_by_field_name() {
        let schema = CollectionSchema {
            fields: vec![],
            indexes: vec![
                IndexInfo {
                    field_name: "other".to_string(),
                    metric_type: Some("L2".to_string()),
                },
                IndexInfo {
                    field_name: "vector".to_string(),
                    metric_type: Some("COSINE".to_string()),
                },
            ],
        };
        assert_eq!(
            MilvusConnector::extract_index_metric(&schema, "vector"),
            Some("cosine".to_string())
        );
    }

    #[test]
    fn test_extract_index_metric_returns_none_when_field_absent() {
        let schema = CollectionSchema {
            fields: vec![],
            indexes: vec![IndexInfo {
                field_name: "other".to_string(),
                metric_type: Some("L2".to_string()),
            }],
        };
        assert_eq!(
            MilvusConnector::extract_index_metric(&schema, "vector"),
            None
        );
    }

    #[test]
    fn test_extract_index_metric_returns_none_when_indexes_empty() {
        let schema = CollectionSchema {
            fields: vec![],
            indexes: vec![],
        };
        assert_eq!(
            MilvusConnector::extract_index_metric(&schema, "vector"),
            None
        );
    }

    #[test]
    fn test_field_schema_dimension_parses_dim_param() {
        let field = FieldSchema {
            name: "v".to_string(),
            field_type: "FloatVector".to_string(),
            is_primary_key: None,
            params: dim_param(512),
        };
        assert_eq!(field.dimension(), 512);
    }

    #[test]
    fn test_field_schema_dimension_returns_zero_when_dim_absent() {
        let field = FieldSchema {
            name: "v".to_string(),
            field_type: "FloatVector".to_string(),
            is_primary_key: None,
            params: vec![],
        };
        assert_eq!(field.dimension(), 0);
    }

    #[test]
    fn test_field_schema_dimension_handles_unparseable_dim() {
        let field = FieldSchema {
            name: "v".to_string(),
            field_type: "FloatVector".to_string(),
            is_primary_key: None,
            params: vec![FieldParam {
                key: "dim".to_string(),
                value: "not_a_number".to_string(),
            }],
        };
        assert_eq!(field.dimension(), 0);
    }
}
