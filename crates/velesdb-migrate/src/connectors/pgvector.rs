//! `PostgreSQL` pgvector and Supabase connectors.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, info};

use super::common::{
    build_numeric_offset_batch, check_response, create_http_client, extract_id_from_value,
    json_type_name,
};
use super::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::config::{PgVectorConfig, SupabaseConfig};
#[cfg(not(feature = "postgres"))]
use crate::error::Error;
use crate::error::Result;

/// pgvector (`PostgreSQL`) source connector.
pub struct PgVectorConnector {
    config: PgVectorConfig,
    #[allow(dead_code)]
    connected: bool,
}

impl PgVectorConnector {
    /// Create a new pgvector connector.
    #[must_use]
    pub fn new(config: PgVectorConfig) -> Self {
        Self {
            config,
            connected: false,
        }
    }
}

#[async_trait]
impl SourceConnector for PgVectorConnector {
    fn source_type(&self) -> &'static str {
        "pgvector"
    }

    async fn connect(&mut self) -> Result<()> {
        info!("pgvector connector requires 'postgres' feature");

        #[cfg(feature = "postgres")]
        {
            // Real implementation with sqlx
            info!("Connecting to PostgreSQL: {}", self.config.table);
            self.connected = true;
            Ok(())
        }

        #[cfg(not(feature = "postgres"))]
        {
            Err(Error::UnsupportedSource(
                "pgvector requires 'postgres' feature. Compile with --features postgres"
                    .to_string(),
            ))
        }
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        #[cfg(feature = "postgres")]
        {
            // Would query pg_catalog for column info
            Ok(SourceSchema {
                source_type: "pgvector".to_string(),
                collection: self.config.table.clone(),
                dimension: 0, // Would be determined from vector column
                total_count: None,
                fields: self
                    .config
                    .payload_columns
                    .iter()
                    .map(|c| FieldInfo {
                        name: c.clone(),
                        field_type: "unknown".to_string(),
                        indexed: false,
                    })
                    .collect(),
                vector_column: Some(self.config.vector_column.clone()),
                id_column: Some(self.config.id_column.clone()),
            })
        }

        #[cfg(not(feature = "postgres"))]
        {
            Err(Error::UnsupportedSource(
                "pgvector requires 'postgres' feature".to_string(),
            ))
        }
    }

    async fn extract_batch(
        &self,
        _offset: Option<serde_json::Value>,
        _batch_size: usize,
    ) -> Result<ExtractedBatch> {
        #[cfg(feature = "postgres")]
        {
            // Would execute:
            // SELECT id, vector, col1, col2 FROM table LIMIT batch_size OFFSET offset
            Ok(ExtractedBatch {
                points: vec![],
                next_offset: None,
                has_more: false,
            })
        }

        #[cfg(not(feature = "postgres"))]
        {
            Err(Error::UnsupportedSource(
                "pgvector requires 'postgres' feature".to_string(),
            ))
        }
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing pgvector connection");
        self.connected = false;
        Ok(())
    }
}

/// Supabase source connector (uses `PostgREST` API).
pub struct SupabaseConnector {
    config: SupabaseConfig,
    client: reqwest::Client,
}

impl SupabaseConnector {
    /// Create a new Supabase connector.
    #[must_use]
    pub fn new(config: SupabaseConfig) -> Self {
        Self {
            config,
            client: create_http_client(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/rest/v1/{}", self.config.url.trim_end_matches('/'), path);
        self.client
            .request(method, &url)
            .header("apikey", &self.config.api_key)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .header("Prefer", "count=exact")
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Reserved for typed row deserialization
struct SupabaseRow {
    #[serde(flatten)]
    data: HashMap<String, serde_json::Value>,
}

#[async_trait]
impl SourceConnector for SupabaseConnector {
    fn source_type(&self) -> &'static str {
        "supabase"
    }

    async fn connect(&mut self) -> Result<()> {
        info!("Connecting to Supabase: {}", self.config.url);

        // Test connection by fetching schema
        let resp = self
            .request(reqwest::Method::GET, &self.config.table)
            .query(&[("limit", "0")])
            .send()
            .await?;

        check_response(resp, "Supabase", "connect").await?;

        info!("Connected to Supabase table: {}", self.config.table);
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        let total_count = self.fetch_total_count().await;

        let resp = self
            .request(reqwest::Method::GET, &self.config.table)
            .query(&[("select", &"*".to_string()), ("limit", &"1".to_string())])
            .send()
            .await?;

        let (dimension, mut fields, detected_vector_col) = if resp.status().is_success() {
            let rows: Vec<HashMap<String, serde_json::Value>> = resp.json().await?;
            rows.first()
                .map(|row| {
                    detect_supabase_schema(row, &self.config.vector_column, &self.config.id_column)
                })
                .unwrap_or_default()
        } else {
            (0, Vec::new(), None)
        };

        if !self.config.payload_columns.is_empty() {
            fields = self
                .config
                .payload_columns
                .iter()
                .map(|c| FieldInfo {
                    name: c.clone(),
                    field_type: "json".to_string(),
                    indexed: false,
                })
                .collect();
        }

        log_supabase_schema(
            &self.config.table,
            dimension,
            total_count,
            &fields,
            &detected_vector_col,
            &self.config.vector_column,
        );

        Ok(SourceSchema {
            source_type: "supabase".to_string(),
            collection: self.config.table.clone(),
            dimension,
            total_count,
            fields,
            vector_column: detected_vector_col,
            id_column: Some(self.config.id_column.clone()),
        })
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let current_offset = offset
            .as_ref()
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        // Build select columns
        let mut select_cols = vec![
            self.config.id_column.clone(),
            self.config.vector_column.clone(),
        ];
        select_cols.extend(self.config.payload_columns.clone());

        debug!(
            "Extracting batch from Supabase, offset={}, limit={}",
            current_offset, batch_size
        );

        let resp = self
            .request(reqwest::Method::GET, &self.config.table)
            .query(&[
                ("select", &select_cols.join(",")),
                ("limit", &batch_size.to_string()),
                ("offset", &current_offset.to_string()),
            ])
            .send()
            .await?;

        let checked = check_response(resp, "Supabase", "extract_batch").await?;

        let rows: Vec<HashMap<String, serde_json::Value>> = checked.json().await?;

        let mut points = Vec::with_capacity(rows.len());

        for mut row in rows {
            let id = extract_id_from_value(row.remove(&self.config.id_column));

            let vector = row
                .remove(&self.config.vector_column)
                .map(|v| parse_pgvector(&v))
                .unwrap_or_default();

            points.push(ExtractedPoint {
                id,
                vector,
                payload: row,
                sparse_vector: None,
            });
        }

        debug!("Extracted {} rows from Supabase", points.len());

        Ok(build_numeric_offset_batch(
            points,
            batch_size,
            current_offset,
        ))
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing Supabase connection");
        Ok(())
    }
}

impl SupabaseConnector {
    /// Fetches the total row count from the content-range header.
    async fn fetch_total_count(&self) -> Option<u64> {
        let resp = self
            .request(reqwest::Method::HEAD, &self.config.table)
            .send()
            .await
            .ok()?;

        resp.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').next_back().and_then(|n| n.parse().ok()))
    }
}

/// Detects vector column, dimension, and metadata fields from a sample row.
fn detect_supabase_schema(
    row: &HashMap<String, serde_json::Value>,
    configured_vector_col: &str,
    id_column: &str,
) -> (usize, Vec<FieldInfo>, Option<String>) {
    let candidates = find_vector_candidates(row);
    let (detected_col, dimension) = pick_best_vector_column(&candidates, configured_vector_col);
    let fields = collect_metadata_fields(row, id_column, detected_col.as_deref());
    (dimension, fields, detected_col)
}

/// Finds columns that look like vector embeddings (dimension > 10).
fn find_vector_candidates(row: &HashMap<String, serde_json::Value>) -> Vec<(String, usize)> {
    row.iter()
        .filter_map(|(col_name, col_value)| {
            let parsed = parse_pgvector(col_value);
            if parsed.len() > 10 {
                Some((col_name.clone(), parsed.len()))
            } else {
                None
            }
        })
        .collect()
}

/// Picks the best vector column from candidates.
fn pick_best_vector_column(
    candidates: &[(String, usize)],
    configured: &str,
) -> (Option<String>, usize) {
    if let Some((name, dim)) = candidates.iter().find(|(n, _)| n == configured) {
        return (Some(name.clone()), *dim);
    }
    if let Some((name, dim)) = candidates.iter().find(|(n, _)| {
        let lower = n.to_lowercase();
        lower.contains("vector") || lower.contains("embedding") || lower.contains("emb")
    }) {
        return (Some(name.clone()), *dim);
    }
    candidates
        .first()
        .map(|(n, d)| (Some(n.clone()), *d))
        .unwrap_or((None, 0))
}

/// Collects non-ID, non-vector metadata fields from a sample row.
fn collect_metadata_fields(
    row: &HashMap<String, serde_json::Value>,
    id_column: &str,
    vector_column: Option<&str>,
) -> Vec<FieldInfo> {
    row.iter()
        .filter(|(col_name, col_value)| {
            if col_name.as_str() == id_column {
                return false;
            }
            if vector_column.is_some_and(|vc| col_name.as_str() == vc) {
                return false;
            }
            let parsed = parse_pgvector(col_value);
            parsed.len() <= 10
        })
        .map(|(col_name, col_value)| FieldInfo {
            name: col_name.clone(),
            field_type: json_type_name(col_value).to_string(),
            indexed: false,
        })
        .collect()
}

/// Logs schema detection results for Supabase.
fn log_supabase_schema(
    table: &str,
    dimension: usize,
    total_count: Option<u64>,
    fields: &[FieldInfo],
    detected_vector_col: &Option<String>,
    configured_vector_col: &str,
) {
    info!(
        "Supabase table '{}': {}D vectors, {:?} rows, {} metadata fields",
        table,
        dimension,
        total_count,
        fields.len()
    );

    if let Some(vec_col) = detected_vector_col {
        if vec_col != configured_vector_col {
            info!(
                "Note: Detected vector column '{}' differs from configured '{}'",
                vec_col, configured_vector_col
            );
        }
    }
}

/// Parse a pgvector string format "[0.1,0.2,0.3]" into Vec<f32>.
fn parse_pgvector(value: &serde_json::Value) -> Vec<f32> {
    match value {
        serde_json::Value::String(s) => {
            // pgvector format: "[0.1,0.2,0.3]"
            let trimmed = s.trim_start_matches('[').trim_end_matches(']');
            trimmed
                .split(',')
                .filter_map(|x| x.trim().parse().ok())
                .collect()
        }
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pgvector_string() {
        let val = serde_json::json!("[0.1,0.2,0.3]");
        let vec = parse_pgvector(&val);
        assert_eq!(vec.len(), 3);
        assert!((vec[0] - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_parse_pgvector_array() {
        let val = serde_json::json!([0.1, 0.2, 0.3]);
        let vec = parse_pgvector(&val);
        assert_eq!(vec.len(), 3);
    }

    #[test]
    fn test_pgvector_connector_new() {
        let config = PgVectorConfig {
            connection_string: "postgres://localhost/test".to_string(),
            table: "embeddings".to_string(),
            vector_column: "embedding".to_string(),
            id_column: "id".to_string(),
            payload_columns: vec!["title".to_string()],
            filter: None,
        };

        let connector = PgVectorConnector::new(config);
        assert_eq!(connector.source_type(), "pgvector");
    }

    #[test]
    fn test_supabase_connector_new() {
        let config = SupabaseConfig {
            url: "https://xxx.supabase.co".to_string(),
            api_key: "test-key".to_string(),
            table: "documents".to_string(),
            vector_column: "embedding".to_string(),
            id_column: "id".to_string(),
            payload_columns: vec![],
        };

        let connector = SupabaseConnector::new(config);
        assert_eq!(connector.source_type(), "supabase");
    }
}
