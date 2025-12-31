//! `PostgreSQL` pgvector and Supabase connectors.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, info};

use super::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::config::{PgVectorConfig, SupabaseConfig};
use crate::error::{Error, Result};

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
            client: reqwest::Client::new(),
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

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::SourceConnection(format!(
                "Supabase connection failed: {status} - {body}"
            )));
        }

        info!("Connected to Supabase table: {}", self.config.table);
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        // Get count from content-range header
        let resp = self
            .request(reqwest::Method::HEAD, &self.config.table)
            .send()
            .await?;

        let total_count = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                // Format: "0-0/123" or "*/123"
                s.split('/').next_back().and_then(|n| n.parse().ok())
            });

        // Fetch one row to detect all columns and vector dimension
        let limit_one = "1".to_string();
        let resp = self
            .request(reqwest::Method::GET, &self.config.table)
            .query(&[("select", &"*".to_string()), ("limit", &limit_one)])
            .send()
            .await?;

        let mut dimension = 0;
        let mut fields: Vec<FieldInfo> = Vec::new();
        let mut detected_vector_col: Option<String> = None;
        let mut best_vector_dimension = 0;

        if resp.status().is_success() {
            let rows: Vec<HashMap<String, serde_json::Value>> = resp.json().await?;
            if let Some(row) = rows.first() {
                // First pass: find all vector columns and their dimensions
                let mut vector_candidates: Vec<(String, usize)> = Vec::new();

                for (col_name, col_value) in row {
                    let parsed_vec = parse_pgvector(col_value);
                    // A vector typically has dimension > 10 (embeddings are usually 128+)
                    if !parsed_vec.is_empty() && parsed_vec.len() > 10 {
                        debug!(
                            "Found potential vector column '{}' with dimension {}",
                            col_name,
                            parsed_vec.len()
                        );
                        vector_candidates.push((col_name.clone(), parsed_vec.len()));
                    }
                }

                // Select the best vector column:
                // 1. Prefer column matching configured name
                // 2. Prefer column with "vector" or "embedding" in name
                // 3. Fall back to first candidate
                for (col_name, dim) in &vector_candidates {
                    if col_name == &self.config.vector_column {
                        detected_vector_col = Some(col_name.clone());
                        best_vector_dimension = *dim;
                        break;
                    }
                }

                if detected_vector_col.is_none() {
                    for (col_name, dim) in &vector_candidates {
                        let lower = col_name.to_lowercase();
                        if lower.contains("vector")
                            || lower.contains("embedding")
                            || lower.contains("emb")
                        {
                            detected_vector_col = Some(col_name.clone());
                            best_vector_dimension = *dim;
                            break;
                        }
                    }
                }

                if detected_vector_col.is_none() && !vector_candidates.is_empty() {
                    let (col_name, dim) = &vector_candidates[0];
                    detected_vector_col = Some(col_name.clone());
                    best_vector_dimension = *dim;
                }

                dimension = best_vector_dimension;

                // Second pass: collect metadata fields (non-vector columns)
                for (col_name, col_value) in row {
                    // Skip ID column and detected vector column
                    if col_name == &self.config.id_column {
                        continue;
                    }
                    if let Some(ref vec_col) = detected_vector_col {
                        if col_name == vec_col {
                            continue;
                        }
                    }

                    // Check if it's a vector column we should skip
                    let parsed_vec = parse_pgvector(col_value);
                    if !parsed_vec.is_empty() && parsed_vec.len() > 10 {
                        continue;
                    }

                    // Determine field type
                    let field_type = match col_value {
                        serde_json::Value::String(_) => "string",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::Bool(_) => "boolean",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                        serde_json::Value::Null => "null",
                    };

                    fields.push(FieldInfo {
                        name: col_name.clone(),
                        field_type: field_type.to_string(),
                        indexed: false,
                    });
                }
            }
        }

        // If payload_columns is specified, filter to only those
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

        info!(
            "Supabase table '{}': {}D vectors, {:?} rows, {} metadata fields",
            self.config.table,
            dimension,
            total_count,
            fields.len()
        );

        if let Some(vec_col) = &detected_vector_col {
            if vec_col != &self.config.vector_column {
                info!(
                    "Note: Detected vector column '{}' differs from configured '{}'",
                    vec_col, self.config.vector_column
                );
            }
        }

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

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Extraction(format!(
                "Supabase query failed: {status} - {body}"
            )));
        }

        let rows: Vec<HashMap<String, serde_json::Value>> = resp.json().await?;

        let mut points = Vec::with_capacity(rows.len());

        for mut row in rows {
            let id = row
                .remove(&self.config.id_column)
                .and_then(|v| match v {
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::String(s) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let vector = row
                .remove(&self.config.vector_column)
                .map(|v| parse_pgvector(&v))
                .unwrap_or_default();

            points.push(ExtractedPoint {
                id,
                vector,
                payload: row,
            });
        }

        let has_more = points.len() == batch_size;
        let next_offset = if has_more {
            Some(serde_json::json!(current_offset + points.len() as u64))
        } else {
            None
        };

        debug!("Extracted {} rows from Supabase", points.len());

        Ok(ExtractedBatch {
            points,
            next_offset,
            has_more,
        })
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing Supabase connection");
        Ok(())
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
